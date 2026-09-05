use anyhow::Result;
use russh::MethodKind;
use russh::keys::ssh_key::rand_core::OsRng;
use russh::keys::*;
use russh::server::Server;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

use wftpd_common::Config;
use wftpd_common::FileLogger;
use wftpd_common::UserManager;
use wftpd_common::server::quota::QuotaCache;

use super::handler::SftpHandler;

#[derive(Clone)]
pub struct SftpServer {
    config: Arc<StdMutex<Config>>,
    user_manager: Arc<StdMutex<UserManager>>,
    file_logger: Arc<StdMutex<FileLogger>>,
    running: Arc<AtomicBool>,
    shutdown_tx: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    closed_rx: Arc<StdMutex<Option<tokio::sync::oneshot::Receiver<()>>>>,
    users_path: PathBuf,
    keys_dir: PathBuf,
    quota_cache: Arc<QuotaCache>,
}

// Implement russh::server::Server trait for SftpServer
impl russh::server::Server for SftpServer {
    type Handler = SftpHandler;

    fn new_client(&mut self, client_addr: Option<SocketAddr>) -> Self::Handler {
        let client_ip = client_addr
            .map(|addr| addr.ip().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        info!(
            "[SFTP SERVER] Creating new handler for client: {}",
            client_ip
        );

        SftpHandler::new(
            Arc::clone(&self.user_manager),
            Arc::clone(&self.file_logger),
            Arc::clone(&self.quota_cache),
            client_ip,
            self.users_path.clone(),
            self.keys_dir.clone(),
        )
    }
}

impl SftpServer {
    pub fn new(
        config: Arc<StdMutex<Config>>,
        user_manager: Arc<StdMutex<UserManager>>,
        file_logger: Arc<StdMutex<FileLogger>>,
    ) -> Self {
        let quota_cache = Arc::new(QuotaCache::new());

        SftpServer {
            config,
            user_manager,
            file_logger,
            running: Arc::new(AtomicBool::new(false)),
            shutdown_tx: Arc::new(Mutex::new(None)),
            closed_rx: Arc::new(StdMutex::new(None)),
            users_path: Config::get_users_path(),
            keys_dir: wftpd_common::paths::keys_dir(),
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
        let quota_cache = Arc::new(QuotaCache::new());

        SftpServer {
            config,
            user_manager,
            file_logger,
            running: Arc::new(AtomicBool::new(false)),
            shutdown_tx: Arc::new(Mutex::new(None)),
            closed_rx: Arc::new(StdMutex::new(None)),
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

    pub async fn start(&self) -> Result<()> {
        let (
            bind_ip,
            sftp_port,
            host_key_path,
            _max_connections,
            max_auth_attempts,
            _auth_timeout,
            idle_timeout,
        ) = {
            match self.config.try_lock() {
                Ok(cfg) => (
                    cfg.sftp.bind_ip.clone(),
                    cfg.sftp.port,
                    cfg.sftp.host_key_path.clone(),
                    cfg.security.max_connections,
                    cfg.sftp.max_auth_attempts as usize,
                    Duration::from_secs(cfg.sftp.auth_timeout),
                    Duration::from_secs(cfg.security.idle_timeout),
                ),
                Err(_) => {
                    self.log_error("Failed to acquire config lock during startup");
                    return Err(anyhow::anyhow!("Failed to acquire config lock"));
                }
            }
        };

        let host_key = Self::load_or_generate_host_key(&host_key_path).await?;

        info!("Loaded host key from: {}", host_key_path);
        info!("Host key algorithm: {:?}", host_key.algorithm());

        // Configure SSH authentication methods
        let mut methods = russh::MethodSet::empty();
        methods.push(MethodKind::Password);
        methods.push(MethodKind::PublicKey);

        // Create russh server configuration
        // 优化连接参数以减少连接等待时间
        let config = russh::server::Config {
            server_id: russh::SshId::Standard(std::borrow::Cow::Borrowed("SSH-2.0-russh_0.59")),
            keys: vec![host_key],
            methods,
            max_auth_attempts,
            inactivity_timeout: Some(idle_timeout),
            // 减少认证拒绝等待时间，避免客户端长时间等待
            auth_rejection_time: Duration::from_secs(1),
            auth_rejection_time_initial: Some(Duration::from_secs(1)),
            ..Default::default()
        };

        let config = Arc::new(config);

        let bind_addr = format!("{}:{}", bind_ip, sftp_port);
        self.log_info(&format!("SFTP server starting on {}", bind_addr));

        // Create a clone of self for the server to run
        let mut server = self.clone();

        // Run the server using russh's built-in server loop
        let addr: SocketAddr = bind_addr
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid bind address: {}", e))?;

        self.running.store(true, Ordering::SeqCst);

        // 注册 shutdown 信号；select 中断 run_on_address 以真正释放监听端口
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        let (closed_tx, closed_rx) = tokio::sync::oneshot::channel();
        {
            let mut tx = self.shutdown_tx.lock().await;
            *tx = Some(shutdown_tx);
        }
        {
            let mut slot = self.closed_rx.lock().unwrap();
            *slot = Some(closed_rx);
        }

        let running_clone = Arc::clone(&self.running);

        tokio::spawn(async move {
            tokio::select! {
                result = server.run_on_address(config, addr) => {
                    match result {
                        Ok(()) => info!("[SFTP] Server stopped normally"),
                        Err(e) => error!("[SFTP] Server error: {}", e),
                    }
                }
                _ = shutdown_rx => {
                    info!("[SFTP] Server shutdown requested, dropping listener");
                }
            }
            // server（以及内部的监听套接字）在此处 drop，随后通知 stop()
            drop(server);
            running_clone.store(false, Ordering::SeqCst);
            let _ = closed_tx.send(());
        });

        Ok(())
    }

    pub async fn stop(&self) -> Result<()> {
        self.running.store(false, Ordering::SeqCst);

        let sender = {
            let mut tx = self.shutdown_tx.lock().await;
            tx.take()
        };
        if let Some(sender) = sender {
            let _ = sender.send(());
        }

        // 等待监听套接字释放，保证随后可以重新 bind 同一端口
        let closed = self.closed_rx.lock().unwrap().take();
        if let Some(rx) = closed {
            let _ = tokio::time::timeout(Duration::from_secs(5), rx).await;
        }

        self.log_info("SFTP server stopped");
        Ok(())
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
                    warn!(
                        "Host key file {:?} has insecure permissions {:o}, should be 0600",
                        path, mode
                    );
                    tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                        .await?;
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
