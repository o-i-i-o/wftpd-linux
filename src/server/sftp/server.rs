use anyhow::Result;
use russh::keys::*;
use russh::keys::ssh_key::rand_core::OsRng;
use russh::MethodKind;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use tokio::sync::Mutex;

use crate::core::config::Config;
use crate::core::logger::Logger;
use crate::core::users::UserManager;
use crate::core::file_logger::FileLogger;

use super::handler::SftpHandler;

#[derive(Clone)]
pub struct SftpServer {
    config: Arc<StdMutex<Config>>,
    user_manager: Arc<StdMutex<UserManager>>,
    logger: Arc<StdMutex<Logger>>,
    file_logger: Arc<StdMutex<FileLogger>>,
    running: Arc<StdMutex<bool>>,
    shutdown_tx: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
}

impl SftpServer {
    pub fn new(
        config: Arc<StdMutex<Config>>,
        user_manager: Arc<StdMutex<UserManager>>,
        logger: Arc<StdMutex<Logger>>,
        file_logger: Arc<StdMutex<FileLogger>>,
    ) -> Self {
        SftpServer {
            config,
            user_manager,
            logger,
            file_logger,
            running: Arc::new(StdMutex::new(false)),
            shutdown_tx: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn start(&self) -> Result<()> {
        let (bind_ip, sftp_port, host_key_path) = {
            let cfg = self.config.lock().unwrap();
            (
                cfg.sftp.bind_ip.clone(),
                cfg.server.sftp_port,
                cfg.sftp.host_key_path.clone(),
            )
        };

        let host_key = Self::load_or_generate_host_key(&host_key_path).await?;

        let mut methods = russh::MethodSet::empty();
        methods.push(MethodKind::Password);
        methods.push(MethodKind::PublicKey);
        let config = russh::server::Config {
            keys: vec![host_key],
            methods,
            ..Default::default()
        };
        let config = Arc::new(config);

        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
        {
            let mut tx = self.shutdown_tx.lock().await;
            *tx = Some(shutdown_tx);
        }

        {
            let mut running = self.running.lock().unwrap();
            *running = true;
        }

        let user_manager_clone = Arc::clone(&self.user_manager);
        let logger_clone = Arc::clone(&self.logger);
        let file_logger_clone = Arc::clone(&self.file_logger);
        let running_clone = Arc::clone(&self.running);

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

        self.logger.lock().unwrap().info("SFTP", &format!("SFTP server started on {}", bind_addr));

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
                                let user_manager = Arc::clone(&user_manager_clone);
                                let logger = Arc::clone(&logger_clone);
                                let file_logger = Arc::clone(&file_logger_clone);
                                let client_ip = peer_addr.ip().to_string();
                                let logger_for_error = Arc::clone(&logger_clone);

                                logger_clone.lock().unwrap().client_action(
                                    "SFTP",
                                    &format!("Client connected from {}", client_ip),
                                    &client_ip,
                                    None,
                                    "CONNECT",
                                );

                                tokio::spawn(async move {
                                    let handler = SftpHandler::new(
                                        user_manager,
                                        logger,
                                        file_logger,
                                        client_ip.clone(),
                                        std::path::PathBuf::from("/etc/wftpg/users.json"),
                                    );

                                    if let Err(e) = russh::server::run_stream(config, socket, handler).await {
                                        let error_msg = format!("{}", e);
                                        if error_msg.contains("Disconnected") || error_msg.contains("Connection reset") {
                                            logger_for_error.lock().unwrap().debug(
                                                "SFTP",
                                                &format!("Client disconnected from {}", peer_addr),
                                            );
                                        } else {
                                            logger_for_error.lock().unwrap().error(
                                                "SFTP",
                                                &format!("SSH connection error from {}: {}", peer_addr, e),
                                            );
                                        }
                                    }
                                });
                            }
                            Err(e) => {
                                logger_clone.lock().unwrap().error(
                                    "SFTP",
                                    &format!("Failed to accept connection: {}", e),
                                );
                            }
                        }
                    }
                }
            }

            {
                let mut running = running_clone.lock().unwrap();
                *running = false;
            }
            logger_clone.lock().unwrap().info("SFTP", "SFTP server stopped");
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
        self.logger.lock().unwrap().info("SFTP", "SFTP server stopped");
    }

    pub fn is_running(&self) -> bool {
        *self.running.lock().unwrap()
    }

    async fn load_or_generate_host_key(path: &str) -> Result<PrivateKey> {
        let path = PathBuf::from(path);

        if path.exists() {
            let key_data = tokio::fs::read_to_string(&path).await?;
            let key = PrivateKey::from_openssh(&key_data)?;
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

        Ok(key)
    }
}
