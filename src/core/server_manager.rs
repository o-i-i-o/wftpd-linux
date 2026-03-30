use std::sync::{Arc, Mutex};
use tracing::info;

use crate::core::config::Config;
use crate::core::users::UserManager;
use crate::core::file_logger::FileLogger;
use crate::server::ftp::FtpServer;
use crate::server::sftp::SftpServer;

#[derive(Clone, Default)]
pub struct ServerManager {
    ftp_server: Arc<Mutex<Option<FtpServer>>>,
    sftp_server: Arc<Mutex<Option<SftpServer>>>,
}

impl ServerManager {
    pub fn new() -> Self {
        Self::default()
    }
    
    pub async fn start_ftp(
        &self,
        config: Arc<Mutex<Config>>,
        user_manager: Arc<Mutex<UserManager>>,
        file_logger: Arc<Mutex<FileLogger>>,
    ) -> anyhow::Result<()> {
        {
            let ftp_server = self.ftp_server.lock().unwrap();
            if ftp_server.is_some() {
                return Ok(());
            }
        }
        
        let server = FtpServer::new(config, user_manager, file_logger);
        server.start().await?;
        
        {
            let mut ftp_server = self.ftp_server.lock().unwrap();
            *ftp_server = Some(server);
        }
        
        info!("FTP server started");
        
        Ok(())
    }
    
    pub async fn stop_ftp(&self) {
        let server = {
            let mut ftp_server = self.ftp_server.lock().unwrap();
            ftp_server.take()
        };
        
        if let Some(srv) = server {
            srv.stop().await;
            info!("FTP server stopped");
        }
    }
    
    pub async fn start_sftp(
        &self,
        config: Arc<Mutex<Config>>,
        user_manager: Arc<Mutex<UserManager>>,
        file_logger: Arc<Mutex<FileLogger>>,
    ) -> anyhow::Result<()> {
        {
            let sftp_server = self.sftp_server.lock().unwrap();
            if sftp_server.is_some() {
                return Ok(());
            }
        }
        
        let server = SftpServer::new(config, user_manager, file_logger);
        server.start().await?;
        
        {
            let mut sftp_server = self.sftp_server.lock().unwrap();
            *sftp_server = Some(server);
        }
        
        info!("SFTP server started successfully");
        
        Ok(())
    }
    
    pub async fn stop_sftp(&self) {
        let server = {
            let mut sftp_server = self.sftp_server.lock().unwrap();
            sftp_server.take()
        };
        
        if let Some(srv) = server {
            srv.stop().await;
            info!("SFTP server stopped");
        }
    }
    
    pub fn is_ftp_running(&self) -> bool {
        self.ftp_server.lock().unwrap().is_some()
    }
    
    pub fn is_sftp_running(&self) -> bool {
        self.sftp_server.lock().unwrap().is_some()
    }
}
