use anyhow::Result;
use russh::keys::*;
use russh::keys::ssh_key::rand_core::OsRng;
use russh::MethodKind;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex as StdMutex;
use std::time::Duration;
use tokio::sync::{Mutex, Semaphore};
use tokio::time::sleep;
use tracing::{info, warn, error, debug};

use crate::core::config::Config;
use crate::core::users::UserManager;
use crate::core::file_logger::FileLogger;
use crate::server::common::quota::QuotaCache;

use super::handler::SftpHandler;

const MAX_ACCEPT_RETRIES: u32 = 10;
const INITIAL_BACKOFF_MS: u64 = 100;
const MAX_BACKOFF_MS: u64 = 5000;

#[derive(Clone)]
pub struct SftpServer {
    config: Arc<StdMutex<Config>>,
    user_manager: Arc<StdMutex<UserManager>>,
    file_logger: Arc<StdMutex<FileLogger>>,
    running: Arc<AtomicBool>,
    shutdown_tx: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    connection_semaphore: Arc<Semaphore>,
    users_path: PathBuf,
    keys_dir: PathBuf,
    quota_cache: Arc<QuotaCache>,
}

impl SftpServer {
    pub fn new(
        config: Arc<StdMutex<Config>>,
        user_manager: Arc<StdMutex<UserManager>>,
        file_logger: Arc<StdMutex<FileLogger>>,
    ) -> Self {
        let max_connections = config.try_lock()
            .map(|c| c.server.max_connections)
            .unwrap_or(100);
        
        let quota_cache = Arc::new(QuotaCache::new());
        
        SftpServer {
            config,
            user_manager,
            file_logger,
            running: Arc::new(AtomicBool::new(false)),
            shutdown_tx: Arc::new(Mutex::new(None)),
            connection_semaphore: Arc::new(Semaphore::new(max_connections)),
            users_path: Config::get_users_path(),
            keys_dir: PathBuf::from("/etc/wftpg/keys"),
            quota_cache,
        }
    }

    pub fn with_paths(
        config: Arc<StdMutex<Config>>,
        user_manager: Arc<StdMutex<UserManager>>,
        file_logger: Arc<StdMutex<FileLogger>>,
        users_path: PathBuf,
        keys_dir: PathBuf,
    ) -> Self {
        let max_connections = config.try_lock()
            .map(|c| c.server.max_connections)
            .unwrap_or(100);
        
        let quota_cache = Arc::new(QuotaCache::new());
        
        SftpServer {
            config,
            user_manager,
            file_logger,
            running: Arc::new(AtomicBool::new(false)),
            shutdown_tx: Arc::new(Mutex::new(None)),
            connection_semaphore: Arc::new(Semaphore::new(max_connections)),
            users_path,
            keys_dir,
            quota_cache,
        }
    }

    fn log_info(&self, message: &str) {
        info!("SFTP: {}", message);
    }

    fn log_error(&self, message: &str) {
        error!("SFTP: {}", message);
    }

    #[allow(dead_code)]
    fn log_warning(&self, message: &str) {
        warn!("SFTP: {}", message);
    }

    #[allow(dead_code)]
    fn log_client_action(&self, action: &str, message: &str, client_ip: &str, username: Option<&str>, log_type: &str) {
        // 使用 file_logger 记录文件操作审计日志
        if let Ok(mut file_log) = self.file_logger.try_lock() {
            file_log.log(crate::core::file_logger::FileLogInfo {
                username: username.unwrap_or("unknown"),
                client_ip,
                operation: action,
                file_path: message,
                file_size: 0,
                protocol: "SFTP",
                success: true,
                message: log_type,
            });
        }
    }

    pub async fn start(&self) -> Result<()> {
        let (bind_ip, sftp_port, host_key_path, _max_connections, max_auth_attempts, auth_timeout, idle_timeout) = {
            match self.config.try_lock() {
                Ok(cfg) => (
                    cfg.sftp.bind_ip.clone(),
                    cfg.server.sftp_port,
                    cfg.sftp.host_key_path.clone(),
                    cfg.server.max_connections,
                    cfg.sftp.max_auth_attempts as usize,
                    Duration::from_secs(cfg.sftp.auth_timeout),
                    Duration::from_secs(cfg.server.idle_timeout),
                ),
                Err(_) => {
                    self.log_error("Failed to acquire config lock during startup");
                    return Err(anyhow::anyhow!("Failed to acquire config lock"));
                }
            }
        };

        let semaphore = Arc::clone(&self.connection_semaphore);

        let host_key = Self::load_or_generate_host_key(&host_key_path).await?;

        let mut methods = russh::MethodSet::empty();
        methods.push(MethodKind::Password);
        methods.push(MethodKind::PublicKey);
        let config = russh::server::Config {
            keys: vec![host_key],
            methods,
            max_auth_attempts,
            inactivity_timeout: Some(idle_timeout),
            auth_rejection_time: auth_timeout,
            ..Default::default()
        };
        let config = Arc::new(config);

        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
        {
            let mut tx = self.shutdown_tx.lock().await;
            *tx = Some(shutdown_tx);
        }

        self.running.store(true, Ordering::SeqCst);

        let user_manager_clone = Arc::clone(&self.user_manager);
        let file_logger_clone = Arc::clone(&self.file_logger);
        let running_clone = Arc::clone(&self.running);
        let config_clone = Arc::clone(&self.config);
        let semaphore_clone = Arc::clone(&semaphore);
        let users_path_clone = self.users_path.clone();
        let keys_dir_clone = self.keys_dir.clone();
        let quota_cache_clone = Arc::clone(&self.quota_cache);

        let bind_addr = format!("{}:{}", bind_ip, sftp_port);
        
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

        self.log_info(&format!("SFTP server started on {}", bind_addr));

        tokio::spawn(async move {
            let mut consecutive_errors = 0u32;
            let mut current_backoff = INITIAL_BACKOFF_MS;

            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => {
                        break;
                    }
                    accept_result = listener.accept() => {
                        match accept_result {
                            Ok((socket, peer_addr)) => {
                                consecutive_errors = 0;
                                current_backoff = INITIAL_BACKOFF_MS;
                                
                                let config = Arc::clone(&config);
                                let user_manager = Arc::clone(&user_manager_clone);
                                let file_logger = Arc::clone(&file_logger_clone);
                                let client_ip = peer_addr.ip().to_string();
                                let config_for_filter = Arc::clone(&config_clone);
                                let semaphore = Arc::clone(&semaphore_clone);
                                let users_path = users_path_clone.clone();
                                let keys_dir = keys_dir_clone.clone();
                                let quota_cache = Arc::clone(&quota_cache_clone);

                                let ip_allowed = match config_for_filter.try_lock() {
                                    Ok(cfg) => cfg.is_ip_allowed(&client_ip),
                                    Err(_) => {
                                        warn!("Failed to check IP filter for {}: config lock failed", client_ip);
                                        false
                                    }
                                };

                                if !ip_allowed {
                                    warn!("Connection rejected from {} by IP filter", client_ip);
                                    continue;
                                }

                                info!("Client connected from {}", client_ip);

                                let permit = match semaphore.clone().try_acquire_owned() {
                                    Ok(p) => p,
                                    Err(_) => {
                                        warn!("Connection rejected from {}: max connections reached", client_ip);
                                        continue;
                                    }
                                };

                                tokio::spawn(async move {
                                    let handler = SftpHandler::new(
                                        user_manager,
                                        file_logger,
                                        quota_cache,
                                        client_ip.clone(),
                                        users_path,
                                        keys_dir,
                                    );

                                    if let Err(e) = russh::server::run_stream(config, socket, handler).await {
                                        let error_msg = format!("{}", e);
                                        if error_msg.contains("Disconnected") || error_msg.contains("Connection reset") {
                                            debug!("Client disconnected from {}", peer_addr);
                                        } else {
                                            error!("SSH connection error from {}: {}", peer_addr, e);
                                        }
                                    }
                                    drop(permit);
                                });
                            }
                            Err(e) => {
                                consecutive_errors += 1;
                                
                                error!("Failed to accept connection (attempt {}): {}", consecutive_errors, e);

                                if consecutive_errors >= MAX_ACCEPT_RETRIES {
                                    error!("Too many consecutive accept errors ({}), stopping server", consecutive_errors);
                                    break;
                                }

                                sleep(Duration::from_millis(current_backoff)).await;
                                current_backoff = (current_backoff * 2).min(MAX_BACKOFF_MS);
                            }
                        }
                    }
                }
            }

            running_clone.store(false, Ordering::SeqCst);
            info!("SFTP server stopped");
        });

        Ok(())
    }

    pub async fn stop(&self) {
        if let Some(tx) = self.shutdown_tx.lock().await.take() {
            let _ = tx.send(());
        }
        self.running.store(false, Ordering::SeqCst);
        self.log_info("SFTP server stop requested");
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    async fn load_or_generate_host_key(path: &str) -> Result<PrivateKey> {
        let path = PathBuf::from(path);

        if path.exists() {
            let key_data = tokio::fs::read_to_string(&path).await?;
            let key = PrivateKey::from_openssh(&key_data)?;
            
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let metadata = tokio::fs::metadata(&path).await?;
                let mode = metadata.permissions().mode() & 0o777;
                if mode != 0o600 {
                    warn!("Host key file {:?} has insecure permissions {:o}, should be 0600", path, mode);
                    tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).await?;
                }
            }
            
            return Ok(key);
        }

        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let mut rng = OsRng;
        let key = PrivateKey::random(&mut rng, Algorithm::Ed25519)?;

        let openssh = key.to_openssh(ssh_key::LineEnding::default())?;
        tokio::fs::write(&path, openssh.to_string()).await?;
        
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).await?;
        }

        let pub_path = path.with_extension("pub");
        let public_key = key.public_key();
        let pub_openssh = public_key.to_openssh()?;
        tokio::fs::write(&pub_path, pub_openssh.to_string()).await?;
        
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&pub_path, std::fs::Permissions::from_mode(0o644)).await?;
        }

        info!("Generated new host key at {:?}", path);

        Ok(key)
    }
}
