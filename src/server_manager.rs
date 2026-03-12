//! Server Manager - Manages FTP and SFTP server instances

use std::sync::{Arc, Mutex};

use crate::config::Config;
use crate::users::UserManager;
use crate::logger::Logger;
use crate::ftp_server::FtpServer;
use crate::sftp_server::SftpServer;

pub struct ServerManager {
    ftp_server: Arc<Mutex<Option<FtpServer>>>,
    sftp_server: Arc<Mutex<Option<SftpServer>>>,
    sftp_runtime: Arc<Mutex<Option<tokio::runtime::Runtime>>>,
}

impl ServerManager {
    pub fn new() -> Self {
        ServerManager {
            ftp_server: Arc::new(Mutex::new(None)),
            sftp_server: Arc::new(Mutex::new(None)),
            sftp_runtime: Arc::new(Mutex::new(None)),
        }
    }
    
    // === FTP Server ===
    
    pub fn start_ftp(
        &self,
        config: Arc<Mutex<Config>>,
        user_manager: Arc<Mutex<UserManager>>,
        logger: Arc<Mutex<Logger>>,
    ) -> anyhow::Result<()> {
        let server = FtpServer::new(config, user_manager, Arc::clone(&logger));
        server.start()?;
        
        let mut ftp_server = self.ftp_server.lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        *ftp_server = Some(server);
        
        Ok(())
    }
    
    pub fn stop_ftp(&self, logger: &Arc<Mutex<Logger>>) {
        let server = {
            let mut ftp_server = match self.ftp_server.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            ftp_server.take()
        };
        
        if let Some(srv) = server {
            srv.stop();
            if let Ok(mut log) = logger.lock() {
                log.info("FTP", "FTP server stopped");
            }
        }
    }
    
    pub fn is_ftp_running(&self) -> bool {
        let ftp_server = match self.ftp_server.lock() {
            Ok(g) => g,
            Err(_) => return false,
        };
        ftp_server.as_ref().is_some_and(|s| s.is_running())
    }
    
    // === SFTP Server ===
    
    pub fn start_sftp(
        &self,
        config: Arc<Mutex<Config>>,
        user_manager: Arc<Mutex<UserManager>>,
        logger: Arc<Mutex<Logger>>,
    ) -> anyhow::Result<()> {
        let worker_threads = std::cmp::min(
            4,
            std::thread::available_parallelism()
                .map(|p| p.get())
                .unwrap_or(2)
        );
        
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(worker_threads)
            .enable_all()
            .thread_keep_alive(std::time::Duration::from_secs(60))
            .max_blocking_threads(8)
            .build()?;
        
        let server = SftpServer::new(config, user_manager, Arc::clone(&logger));
        
        {
            let mut sftp_server = self.sftp_server.lock()
                .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
            *sftp_server = Some(server.clone());
        }
        
        let logger_for_async = Arc::clone(&logger);
        runtime.spawn(async move {
            if let Err(e) = server.start().await {
                eprintln!("SFTP server error: {}", e);
                if let Ok(mut log) = logger_for_async.lock() {
                    log.error("SFTP", &format!("SFTP server error: {}", e));
                }
            }
        });
        
        {
            let mut sftp_runtime = self.sftp_runtime.lock()
                .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
            *sftp_runtime = Some(runtime);
        }
        
        if let Ok(mut log) = logger.lock() {
            log.info("SFTP", "SFTP server starting...");
        }
        
        Ok(())
    }
    
    pub fn stop_sftp(&self, logger: &Arc<Mutex<Logger>>) {
        let runtime = {
            let mut sftp_runtime = match self.sftp_runtime.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            sftp_runtime.take()
        };
        
        let server = {
            let mut sftp_server = match self.sftp_server.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            sftp_server.take()
        };
        
        if let (Some(rt), Some(srv)) = (runtime, server) {
            rt.spawn(async move {
                srv.stop().await
            });
            rt.shutdown_background();
        }
        
        if let Ok(mut log) = logger.lock() {
            log.info("SFTP", "SFTP server stopped");
        }
    }
    
    pub fn is_sftp_running(&self) -> bool {
        let sftp_server = match self.sftp_server.lock() {
            Ok(g) => g,
            Err(_) => return false,
        };
        sftp_server.as_ref().is_some_and(|s| s.is_running())
    }
}

impl Default for ServerManager {
    fn default() -> Self {
        Self::new()
    }
}
