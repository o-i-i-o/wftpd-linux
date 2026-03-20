mod session;
mod commands;
mod passive;
mod transfer;
mod tls;

mod ftps_listener;

use anyhow::Result;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::core::config::{Config, get_program_data_path};
use crate::core::logger::Logger;
use crate::core::users::UserManager;
use crate::core::file_logger::FileLogger;
use crate::core::quota::QuotaManager;

use crate::core::ftp_server::tls::TlsConfig;

pub struct FtpServer {
    config: Arc<std::sync::Mutex<Config>>,
    user_manager: Arc<std::sync::Mutex<UserManager>>,
    logger: Arc<std::sync::Mutex<Logger>>,
    file_logger: Arc<std::sync::Mutex<FileLogger>>,
    quota_manager: Arc<QuotaManager>,
    running: Arc<std::sync::Mutex<bool>>,
    shutdown_tx: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    ftps_shutdown_tx: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    tls_config: Arc<std::sync::Mutex<Option<TlsConfig>>>,
}

impl FtpServer {
    pub fn new(
        config: Arc<std::sync::Mutex<Config>>,
        user_manager: Arc<std::sync::Mutex<UserManager>>,
        logger: Arc<std::sync::Mutex<Logger>>,
        file_logger: Arc<std::sync::Mutex<FileLogger>>,
    ) -> Self {
        let tls_config = {
            match config.lock() {
                Ok(cfg) => {
                    if cfg.ftp.ftps.enabled {
                        let cert_path = cfg.ftp.ftps.cert_path.as_deref();
                        let key_path = cfg.ftp.ftps.key_path.as_deref();
                        Some(TlsConfig::new(cert_path, key_path, cfg.ftp.ftps.require_ssl))
                    } else {
                        None
                    }
                }
                Err(e) => {
                    log::error!("获取配置锁失败: {}", e);
                    None
                }
            }
        };
        
        let quota_manager = QuotaManager::new(&get_program_data_path());
        
        FtpServer {
            config,
            user_manager,
            logger,
            file_logger,
            quota_manager: Arc::new(quota_manager),
            running: Arc::new(std::sync::Mutex::new(false)),
            shutdown_tx: Arc::new(Mutex::new(None)),
            ftps_shutdown_tx: Arc::new(Mutex::new(None)),
            tls_config: Arc::new(std::sync::Mutex::new(tls_config)),
        }
    }

    pub async fn start(&self) -> Result<()> {
        let (bind_ip, ftp_port, warnings, ftps_enabled, ftps_implicit, ftps_port) = {
            let cfg = match self.config.lock() {
                Ok(guard) => guard,
                Err(e) => return Err(anyhow::anyhow!("获取配置锁失败: {}", e)),
            };
            let warnings = cfg.validate_paths();
            (
                cfg.ftp.bind_ip.clone(), 
                cfg.server.ftp_port, 
                warnings,
                cfg.ftp.ftps.enabled,
                cfg.ftp.ftps.implicit_ssl,
                cfg.ftp.ftps.implicit_ssl_port,
            )
        };

        if !warnings.is_empty() {
            for warning in &warnings {
                log::error!("配置验证失败: {}", warning);
            }
            return Err(anyhow::anyhow!("配置路径验证失败: {}", warnings.join("; ")));
        }

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
            let mut running = match self.running.lock() {
                Ok(guard) => guard,
                Err(e) => return Err(anyhow::anyhow!("获取运行状态锁失败: {}", e)),
            };
            *running = true;
        }

        let config = Arc::clone(&self.config);
        let user_manager = Arc::clone(&self.user_manager);
        let logger = Arc::clone(&self.logger);
        let file_logger = Arc::clone(&self.file_logger);
        let quota_manager = Arc::clone(&self.quota_manager);
        let running_clone = Arc::clone(&self.running);

        if let Ok(mut log) = self.logger.lock() {
            log.info("FTP", &format!("FTP server started on {}", bind_addr));
        }

        if ftps_enabled && ftps_implicit {
            let tls_cfg_opt = self.tls_config.lock().unwrap().clone();
            if let Some(tls_cfg) = tls_cfg_opt {
                let (ftps_shutdown_tx, ftps_shutdown_rx) = tokio::sync::oneshot::channel();
                {
                    let mut tx = self.ftps_shutdown_tx.lock().await;
                    *tx = Some(ftps_shutdown_tx);
                }
                
                let ftps_bind_addr = format!("{}:{}", bind_ip, ftps_port);
                if let Ok(mut log) = self.logger.lock() {
                    log.info("FTPS", &format!("FTPS implicit SSL server starting on {}", ftps_bind_addr));
                }
                
                let config_clone = Arc::clone(&config);
                let user_manager_clone = Arc::clone(&user_manager);
                let logger_clone = Arc::clone(&logger);
                let file_logger_clone = Arc::clone(&file_logger);
                let quota_manager_clone = Arc::clone(&quota_manager);
                
                tokio::spawn(async move {
                    if let Err(e) = ftps_listener::start_ftps_implicit_server(
                        config_clone,
                        user_manager_clone,
                        logger_clone,
                        file_logger_clone,
                        quota_manager_clone,
                        tls_cfg,
                        ftps_shutdown_rx,
                    ).await {
                        log::error!("FTPS implicit SSL server error: {}", e);
                    }
                });
            }
        }

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => {
                        break;
                    }
                    accept_result = listener.accept() => {
                        match accept_result {
                            Ok((socket, peer_addr)) => {
                                let config = Arc::clone(&config);
                                let user_manager = Arc::clone(&user_manager);
                                let logger = Arc::clone(&logger);
                                let file_logger = Arc::clone(&file_logger);
                                let quota_manager = Arc::clone(&quota_manager);
                                let client_ip = peer_addr.ip().to_string();

                                if let Ok(mut log) = logger.lock() {
                                    log.client_action(
                                        "FTP",
                                        &format!("Client connected from {}", client_ip),
                                        &client_ip,
                                        None,
                                        "CONNECT",
                                    );
                                }

                                tokio::spawn(async move {
                                    if let Err(e) = session::handle_session(
                                        socket,
                                        config,
                                        user_manager,
                                        logger,
                                        file_logger,
                                        quota_manager,
                                        client_ip,
                                    ).await {
                                        log::debug!("FTP session error: {}", e);
                                    }
                                });
                            }
                            Err(e) => {
                                log::warn!("Failed to accept FTP connection: {}", e);
                            }
                        }
                    }
                }
            }

            if let Ok(mut running) = running_clone.lock() {
                *running = false;
            }
        });

        Ok(())
    }

    pub async fn stop(&self) {
        {
            let mut tx = self.shutdown_tx.lock().await;
            if let Some(sender) = tx.take() {
                let _ = sender.send(());
            }
        }
        {
            let mut running = match self.running.lock() {
                Ok(guard) => guard,
                Err(e) => {
                    log::error!("获取运行状态锁失败: {}", e);
                    return;
                }
            };
            *running = false;
        }
        if let Ok(mut logger) = self.logger.lock() {
            logger.info("FTP", "FTP server stopped");
        }
    }

    pub fn is_running(&self) -> bool {
        match self.running.lock() {
            Ok(guard) => *guard,
            Err(_) => false,
        }
    }
}
