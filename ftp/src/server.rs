use anyhow::Result;
use rustls::ServerConfig;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use tokio::io::AsyncWriteExt;
use tokio::sync::{Mutex, Semaphore};
use tracing::{error, info, warn};

use wftpd_common::Config;
use wftpd_common::FileLogger;
use wftpd_common::UserManager;
use wftpd_common::server::login_tracker::LoginTracker;
use wftpd_common::server::quota::QuotaCache;

use super::data_connection::PassiveListenerMap;
use super::handler::{FtpSession, FtpSessionConfig};
use super::rate_limit::RateLimiter;
use super::tls::TlsConfig;

pub struct FtpServer {
    config: Arc<StdMutex<Config>>,
    user_manager: Arc<StdMutex<UserManager>>,
    file_logger: Arc<StdMutex<FileLogger>>,
    running: Arc<StdMutex<bool>>,
    shutdown_tx: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    closed_rx: Arc<StdMutex<Option<tokio::sync::oneshot::Receiver<()>>>>,
    passive_listeners: PassiveListenerMap,
    rate_limiter: Arc<RateLimiter>,
    tls_config: Option<TlsConfig>,
    tls_server_config: Option<Arc<ServerConfig>>,
    connection_semaphore: Arc<Semaphore>,
    login_tracker: Arc<LoginTracker>,
    quota_cache: Arc<QuotaCache>,
}

impl FtpServer {
    pub fn new(
        config: Arc<StdMutex<Config>>,
        user_manager: Arc<StdMutex<UserManager>>,
        file_logger: Arc<StdMutex<FileLogger>>,
    ) -> Self {
        let rate_limiter = Arc::new(RateLimiter::new(10, 60, 100));

        let (tls_config, tls_server_config, max_connections, max_login_attempts, ban_duration) = {
            let cfg = config.lock().unwrap();
            let tls = if cfg.security.require_ssl {
                if let (Some(cert_path), Some(key_path)) =
                    (&cfg.security.cert_path, &cfg.security.key_path)
                {
                    let tls_cfg = TlsConfig::new(true, cert_path.clone(), key_path.clone(), true);
                    let server_cfg = tls_cfg.load_server_config().ok();
                    (Some(tls_cfg), server_cfg)
                } else {
                    (None, None)
                }
            } else {
                (None, None)
            };
            (
                tls.0,
                tls.1,
                cfg.security.max_connections,
                cfg.security.max_login_attempts,
                cfg.security.ban_duration,
            )
        };

        let login_tracker = Arc::new(LoginTracker::new(max_login_attempts, ban_duration));
        let quota_cache = Arc::new(QuotaCache::new());

        FtpServer {
            config,
            user_manager,
            file_logger,
            running: Arc::new(StdMutex::new(false)),
            shutdown_tx: Arc::new(Mutex::new(None)),
            closed_rx: Arc::new(StdMutex::new(None)),
            passive_listeners: Arc::new(Mutex::new(HashMap::new())),
            rate_limiter,
            tls_config,
            tls_server_config,
            connection_semaphore: Arc::new(Semaphore::new(max_connections)),
            login_tracker,
            quota_cache,
        }
    }

    pub fn with_tls(
        config: Arc<StdMutex<Config>>,
        user_manager: Arc<StdMutex<UserManager>>,
        file_logger: Arc<StdMutex<FileLogger>>,
        tls_config: TlsConfig,
    ) -> Result<Self> {
        let rate_limiter = Arc::new(RateLimiter::new(10, 60, 100));
        let tls_server_config = tls_config.load_server_config()?;
        let (max_connections, max_login_attempts, ban_duration) = {
            let cfg = config.lock().unwrap();
            (
                cfg.security.max_connections,
                cfg.security.max_login_attempts,
                cfg.security.ban_duration,
            )
        };
        let login_tracker = Arc::new(LoginTracker::new(max_login_attempts, ban_duration));
        let quota_cache = Arc::new(QuotaCache::new());

        Ok(FtpServer {
            config,
            user_manager,
            file_logger,
            running: Arc::new(StdMutex::new(false)),
            shutdown_tx: Arc::new(Mutex::new(None)),
            closed_rx: Arc::new(StdMutex::new(None)),
            passive_listeners: Arc::new(Mutex::new(HashMap::new())),
            rate_limiter,
            tls_config: Some(tls_config),
            tls_server_config: Some(tls_server_config),
            connection_semaphore: Arc::new(Semaphore::new(max_connections)),
            login_tracker,
            quota_cache,
        })
    }

    pub async fn start(&self) -> Result<()> {
        let (bind_ip, ftp_port) = {
            let cfg = self.config.lock().unwrap();
            (cfg.ftp.bind_ip.clone(), cfg.ftp.port)
        };
        let bind_addr = format!("{}:{}", bind_ip, ftp_port);

        let listener = {
            use socket2::{Domain, Protocol, SockAddr, Socket, Type};
            let socket = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))?;
            socket.set_reuse_address(true)?;
            socket.set_nonblocking(true)?;
            let addr: std::net::SocketAddr = bind_addr
                .parse()
                .map_err(|e| anyhow::anyhow!("Invalid bind address: {}", e))?;
            socket.bind(&SockAddr::from(addr))?;
            socket.listen(128)?;
            tokio::net::TcpListener::from_std(socket.into())
                .map_err(|e| anyhow::anyhow!("Failed to create tokio listener: {}", e))?
        };

        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
        let (closed_tx, closed_rx) = tokio::sync::oneshot::channel();
        {
            let mut tx = self.shutdown_tx.lock().await;
            *tx = Some(shutdown_tx);
        }
        {
            let mut slot = self.closed_rx.lock().unwrap();
            *slot = Some(closed_rx);
        }

        {
            let mut running = self.running.lock().unwrap();
            *running = true;
        }

        info!(bind_addr = %bind_addr, tls = self.tls_config.is_some(), "FTP 服务已启动");

        let config = Arc::clone(&self.config);
        let user_manager = Arc::clone(&self.user_manager);
        let file_logger = Arc::clone(&self.file_logger);
        let running = Arc::clone(&self.running);
        let passive_listeners = Arc::clone(&self.passive_listeners);
        let rate_limiter = Arc::clone(&self.rate_limiter);
        let tls_config = self.tls_config.clone();
        let tls_server_config = self.tls_server_config.clone();
        let semaphore = Arc::clone(&self.connection_semaphore);
        let login_tracker = Arc::clone(&self.login_tracker);
        let quota_cache = Arc::clone(&self.quota_cache);

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => {
                        break;
                    }
                    accept_result = listener.accept() => {
                        match accept_result {
                            Ok((mut stream, peer_addr)) => {
                                let config = Arc::clone(&config);
                                let user_manager = Arc::clone(&user_manager);
                                let file_logger = Arc::clone(&file_logger);
                                let passive_listeners = Arc::clone(&passive_listeners);
                                let rate_limiter = Arc::clone(&rate_limiter);
                                let tls_config = tls_config.clone();
                                let tls_server_config = tls_server_config.clone();
                                let client_ip = peer_addr.ip().to_string();
                                let semaphore = Arc::clone(&semaphore);
                                let login_tracker = Arc::clone(&login_tracker);
                                let quota_cache = Arc::clone(&quota_cache);

                                {
                                    let cfg = config.lock().unwrap();
                                    if !cfg.is_ip_allowed(&client_ip) {
                                        warn!("Connection rejected from {} by IP filter", client_ip);
                                        continue;
                                    }
                                }

                                let permit = match semaphore.clone().try_acquire_owned() {
                                    Ok(p) => p,
                                    Err(_) => {
                                        warn!("Connection rejected from {}: max connections reached", client_ip);
                                        let _ = stream.write_all(b"421 Service not available, too many connections\r\n").await;
                                        continue;
                                    }
                                };

                                info!("Client connected from {}", client_ip);

                                tokio::spawn(async move {
                                    let session_config = FtpSessionConfig {
                                        config,
                                        user_manager,
                                        file_logger,
                                        passive_listeners,
                                        rate_limiter,
                                        tls_config,
                                        tls_server_config,
                                        login_tracker,
                                        quota_cache,
                                    };
                                    match FtpSession::new(stream, session_config) {
                                        Ok(mut session) => {
                                            if let Err(e) = session.run().await {
                                                error!("Session error from {}: {}", peer_addr, e);
                                            }
                                        }
                                        Err(e) => {
                                            error!("Failed to create session from {}: {}", peer_addr, e);
                                        }
                                    }
                                    drop(permit);
                                });
                            }
                            Err(e) => {
                                error!("Failed to accept connection: {}", e);
                            }
                        }
                    }
                }
            }

            drop(listener);

            {
                let mut running = running.lock().unwrap();
                *running = false;
            }

            // 通知 stop() 监听套接字已释放，可以安全重新 bind
            let _ = closed_tx.send(());

            info!("FTP server stopped");
        });

        Ok(())
    }

    pub async fn stop(&self) {
        if let Some(tx) = self.shutdown_tx.lock().await.take() {
            let _ = tx.send(());
        }

        // 等待 accept 循环退出并释放监听套接字（重启时避免 Address already in use）
        let closed = self.closed_rx.lock().unwrap().take();
        if let Some(rx) = closed {
            let _ = tokio::time::timeout(std::time::Duration::from_secs(5), rx).await;
        }

        {
            let mut running = self.running.lock().unwrap();
            *running = false;
        }

        let mut listeners = self.passive_listeners.lock().await;
        listeners.clear();

        info!("FTP 服务已停止");
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
