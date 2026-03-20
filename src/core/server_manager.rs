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
}

impl ServerManager {
    pub fn new() -> Self {
        ServerManager {
            ftp_server: Arc::new(Mutex::new(None)),
            sftp_server: Arc::new(Mutex::new(None)),
        }
    }
    
    pub async fn start_ftp(
        &self,
        config: Arc<Mutex<Config>>,
        user_manager: Arc<Mutex<UserManager>>,
        logger: Arc<Mutex<Logger>>,
        file_logger: Arc<Mutex<FileLogger>>,
    ) -> anyhow::Result<()> {
        {
            let ftp_server = self.ftp_server.lock().unwrap();
            if ftp_server.is_some() {
                return Ok(());
            }
        }
        
        let server = FtpServer::new(config, user_manager, logger, file_logger);
        server.start().await?;
        
        {
            let mut ftp_server = self.ftp_server.lock().unwrap();
            *ftp_server = Some(server);
        }
        
        Ok(())
    }
    
    pub async fn stop_ftp(&self, logger: &Arc<Mutex<Logger>>) {
        let server = {
            let mut ftp_server = self.ftp_server.lock().unwrap();
            ftp_server.take()
        };
        
        if let Some(srv) = server {
            srv.stop().await;
            if let Ok(mut log) = logger.lock() {
                log.info("FTP", "FTP server stopped");
            }
        }
    }
    
    pub fn is_ftp_running(&self) -> bool {
        let ftp_server = self.ftp_server.lock().unwrap();
        ftp_server.as_ref().is_some_and(|s| s.is_running())
    }
    
    pub async fn start_sftp(
        &self,
        config: Arc<Mutex<Config>>,
        user_manager: Arc<Mutex<UserManager>>,
        logger: Arc<Mutex<Logger>>,
        file_logger: Arc<Mutex<FileLogger>>,
    ) -> anyhow::Result<()> {
        {
            let sftp_server = self.sftp_server.lock().unwrap();
            if sftp_server.is_some() {
                return Ok(());
            }
        }
        
        let server = SftpServer::new(config, user_manager, Arc::clone(&logger), file_logger);
        server.start().await?;
        
        {
            let mut sftp_server = self.sftp_server.lock().unwrap();
            *sftp_server = Some(server);
        }
        
        if let Ok(mut log) = logger.lock() {
            log.info("SFTP", "SFTP server started successfully");
        }
        
        Ok(())
    }
    
    pub async fn stop_sftp(&self, logger: &Arc<Mutex<Logger>>) {
        let server = {
            let mut sftp_server = self.sftp_server.lock().unwrap();
            sftp_server.take()
        };
        
        if let Some(srv) = server {
            srv.stop().await;
            if let Ok(mut log) = logger.lock() {
                log.info("SFTP", "SFTP server stopped");
            }
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
