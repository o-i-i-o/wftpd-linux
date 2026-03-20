use anyhow::Result;
use rustls::ServerConfig;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use tokio::sync::Mutex;

use crate::core::config::Config;
use crate::core::logger::Logger;
use crate::core::users::UserManager;
use crate::core::file_logger::FileLogger;

use super::handler::FtpSession;
use super::rate_limit::RateLimiter;
use super::data_connection::PassiveListenerMap;
use super::tls::TlsConfig;

pub struct FtpServer {
    config: Arc<StdMutex<Config>>,
    user_manager: Arc<StdMutex<UserManager>>,
    logger: Arc<StdMutex<Logger>>,
    file_logger: Arc<StdMutex<FileLogger>>,
    running: Arc<StdMutex<bool>>,
    shutdown_tx: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    passive_listeners: PassiveListenerMap,
    rate_limiter: Arc<RateLimiter>,
    tls_config: Option<TlsConfig>,
    tls_server_config: Option<Arc<ServerConfig>>,
}

impl FtpServer {
    pub fn new(
        config: Arc<StdMutex<Config>>,
        user_manager: Arc<StdMutex<UserManager>>,
        logger: Arc<StdMutex<Logger>>,
        file_logger: Arc<StdMutex<FileLogger>>,
    ) -> Self {
        let rate_limiter = Arc::new(RateLimiter::new(10, 60, 100));
        
        let (tls_config, tls_server_config) = {
            let cfg = config.lock().unwrap();
            if cfg.security.require_ssl {
                if let (Some(cert_path), Some(key_path)) = (&cfg.security.cert_path, &cfg.security.key_path) {
                    let tls_cfg = TlsConfig::new(true, cert_path.clone(), key_path.clone(), true);
                    let server_cfg = tls_cfg.load_server_config().ok();
                    (Some(tls_cfg), server_cfg)
                } else {
                    (None, None)
                }
            } else {
                (None, None)
            }
        };
        
        FtpServer {
            config,
            user_manager,
            logger,
            file_logger,
            running: Arc::new(StdMutex::new(false)),
            shutdown_tx: Arc::new(Mutex::new(None)),
            passive_listeners: Arc::new(Mutex::new(HashMap::new())),
            rate_limiter,
            tls_config,
            tls_server_config,
        }
    }

    pub fn with_tls(
        config: Arc<StdMutex<Config>>,
        user_manager: Arc<StdMutex<UserManager>>,
        logger: Arc<StdMutex<Logger>>,
        file_logger: Arc<StdMutex<FileLogger>>,
        tls_config: TlsConfig,
    ) -> Result<Self> {
        let rate_limiter = Arc::new(RateLimiter::new(10, 60, 100));
        let tls_server_config = tls_config.load_server_config()?;
        
        Ok(FtpServer {
            config,
            user_manager,
            logger,
            file_logger,
            running: Arc::new(StdMutex::new(false)),
            shutdown_tx: Arc::new(Mutex::new(None)),
            passive_listeners: Arc::new(Mutex::new(HashMap::new())),
            rate_limiter,
            tls_config: Some(tls_config),
            tls_server_config: Some(tls_server_config),
        })
    }

    pub async fn start(&self) -> Result<()> {
        let (bind_ip, ftp_port) = {
            let cfg = self.config.lock().unwrap();
            (cfg.server.bind_ip.clone(), cfg.server.ftp_port)
        };
        let bind_addr = format!("{}:{}", bind_ip, ftp_port);
        
        let listener = {
            use socket2::{Domain, Protocol, Socket, Type, SockAddr};
            let socket = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))?;
            socket.set_reuse_address(true)?;
            socket.set_nonblocking(true)?;
            let addr: std::net::SocketAddr = bind_addr.parse()
                .map_err(|e| anyhow::anyhow!("Invalid bind address: {}", e))?;
            socket.bind(&SockAddr::from(addr))?;
            socket.listen(128)?;
            tokio::net::TcpListener::from_std(socket.into())
                .map_err(|e| anyhow::anyhow!("Failed to create tokio listener: {}", e))?
        };
        
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
        {
            let mut tx = self.shutdown_tx.lock().await;
            *tx = Some(shutdown_tx);
        }
        
        {
            let mut running = self.running.lock().unwrap();
            *running = true;
        }

        let tls_info = if self.tls_config.is_some() {
            " (TLS enabled)"
        } else {
            ""
        };
        self.logger.lock().unwrap().info("FTP", &format!("FTP server started on {}{}", bind_addr, tls_info));

        let config = Arc::clone(&self.config);
        let user_manager = Arc::clone(&self.user_manager);
        let logger = Arc::clone(&self.logger);
        let file_logger = Arc::clone(&self.file_logger);
        let running = Arc::clone(&self.running);
        let passive_listeners = Arc::clone(&self.passive_listeners);
        let rate_limiter = Arc::clone(&self.rate_limiter);
        let tls_config = self.tls_config.clone();
        let tls_server_config = self.tls_server_config.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => {
                        break;
                    }
                    accept_result = listener.accept() => {
                        match accept_result {
                            Ok((stream, peer_addr)) => {
                                let config = Arc::clone(&config);
                                let user_manager = Arc::clone(&user_manager);
                                let logger_for_session = Arc::clone(&logger);
                                let logger_for_error = Arc::clone(&logger);
                                let file_logger = Arc::clone(&file_logger);
                                let passive_listeners = Arc::clone(&passive_listeners);
                                let rate_limiter = Arc::clone(&rate_limiter);
                                let tls_config = tls_config.clone();
                                let tls_server_config = tls_server_config.clone();
                                let client_ip = peer_addr.ip().to_string();
                                
                                {
                                    let cfg = config.lock().unwrap();
                                    if !cfg.is_ip_allowed(&client_ip) {
                                        logger.lock().unwrap().warning(
                                            "FTP",
                                            &format!("Connection rejected from {} by IP filter", client_ip),
                                        );
                                        continue;
                                    }
                                }
                                
                                logger.lock().unwrap().client_action(
                                    "FTP",
                                    &format!("Client connected from {}", client_ip),
                                    &client_ip,
                                    None,
                                    "CONNECT",
                                );

                                tokio::spawn(async move {
                                    match FtpSession::new(
                                        stream,
                                        config,
                                        user_manager,
                                        logger_for_session,
                                        file_logger,
                                        passive_listeners,
                                        rate_limiter,
                                        tls_config,
                                        tls_server_config,
                                    ) {
                                        Ok(mut session) => {
                                            if let Err(e) = session.run().await {
                                                logger_for_error.lock().unwrap().error(
                                                    "FTP",
                                                    &format!("Session error from {}: {}", peer_addr, e),
                                                );
                                            }
                                        }
                                        Err(e) => {
                                            logger_for_error.lock().unwrap().error(
                                                "FTP",
                                                &format!("Failed to create session from {}: {}", peer_addr, e),
                                            );
                                        }
                                    }
                                });
                            }
                            Err(e) => {
                                logger.lock().unwrap().error(
                                    "FTP",
                                    &format!("Failed to accept connection: {}", e),
                                );
                            }
                        }
                    }
                }
            }
            
            {
                let mut running = running.lock().unwrap();
                *running = false;
            }
            
            logger.lock().unwrap().info("FTP", "FTP server stopped");
        });

        Ok(())
    }

    pub async fn stop(&self) {
        if let Some(tx) = self.shutdown_tx.lock().await.take() {
            let _ = tx.send(());
        }
        
        {
            let mut running = self.running.lock().unwrap();
            *running = false;
        }
        
        let mut listeners = self.passive_listeners.lock().await;
        listeners.clear();
        
        self.logger.lock().unwrap().info("FTP", "FTP server stopped");
    }

    pub fn is_running(&self) -> bool {
        *self.running.lock().unwrap()
    }

    pub fn get_connection_stats(&self) -> (usize, u32) {
        self.rate_limiter.get_stats()
    }

    pub fn is_tls_enabled(&self) -> bool {
        self.tls_config.is_some() && self.tls_server_config.is_some()
    }
}
