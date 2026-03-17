use std::sync::{Arc, Mutex};

use crate::core::config::Config;
use crate::core::users::UserManager;
use crate::core::logger::Logger;
use crate::core::file_logger::FileLogger;
use crate::server::ftp::FtpServer;
use crate::server::sftp::SftpServer;

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
    
    pub fn start_ftp(
        &self,
        config: Arc<Mutex<Config>>,
        user_manager: Arc<Mutex<UserManager>>,
        logger: Arc<Mutex<Logger>>,
        file_logger: Arc<Mutex<FileLogger>>,
    ) -> anyhow::Result<()> {
        let ftp_server = self.ftp_server.lock().unwrap();
        if ftp_server.is_some() {
            return Ok(());
        }
        drop(ftp_server);
        
        let server = FtpServer::new(config, user_manager, logger, file_logger);
        server.start()?;
        
        {
            let mut srv = self.ftp_server.lock().unwrap();
            *srv = Some(server);
        }
        
        Ok(())
    }
    
    pub fn stop_ftp(&self, logger: &Arc<Mutex<Logger>>) {
        let mut ftp_server = self.ftp_server.lock().unwrap();
        if let Some(server) = ftp_server.take() {
            server.stop();
            if let Ok(mut log) = logger.lock() {
                log.info("FTP", "FTP server stopped");
            }
        }
    }
    
    pub fn is_ftp_running(&self) -> bool {
        let ftp_server = self.ftp_server.lock().unwrap();
        ftp_server.as_ref().is_some_and(|s| s.is_running())
    }
    
    pub fn start_sftp(
        &self,
        config: Arc<Mutex<Config>>,
        user_manager: Arc<Mutex<UserManager>>,
        logger: Arc<Mutex<Logger>>,
        file_logger: Arc<Mutex<FileLogger>>,
    ) -> anyhow::Result<()> {
        let sftp_server = self.sftp_server.lock().unwrap();
        if sftp_server.is_some() {
            return Ok(());
        }
        drop(sftp_server);
        
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?;
        
        let server = SftpServer::new(config, user_manager, Arc::clone(&logger), file_logger);
        
        runtime.block_on(async {
            server.start().await
        })?;
        
        {
            let mut rt = self.sftp_runtime.lock().unwrap();
            *rt = Some(runtime);
        }
        
        {
            let mut srv = self.sftp_server.lock().unwrap();
            *srv = Some(server);
        }
        
        if let Ok(mut log) = logger.lock() {
            log.info("SFTP", "SFTP server started successfully");
        }
        
        Ok(())
    }
    
    pub fn stop_sftp(&self, logger: &Arc<Mutex<Logger>>) {
        let server = {
            let mut sftp_server = self.sftp_server.lock().unwrap();
            sftp_server.take()
        };
        
        if let Some(srv) = server {
            let runtime = {
                let mut sftp_runtime = self.sftp_runtime.lock().unwrap();
                sftp_runtime.take()
            };
            
            if let Some(rt) = runtime {
                rt.block_on(async {
                    srv.stop().await
                });
                rt.shutdown_timeout(std::time::Duration::from_secs(5));
            }
        }
        
        if let Ok(mut log) = logger.lock() {
            log.info("SFTP", "SFTP server stopped");
        }
    }
    
    pub fn is_sftp_running(&self) -> bool {
        let sftp_server = self.sftp_server.lock().unwrap();
        sftp_server.as_ref().is_some_and(|s| s.is_running())
    }
}

impl Default for ServerManager {
    fn default() -> Self {
        Self::new()
    }
}
