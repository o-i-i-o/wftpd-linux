#![allow(dead_code)]

pub mod config;
pub mod users;
pub mod logger;
pub mod ftp_server;
pub mod sftp_server;
pub mod service;

use std::sync::{Arc, Mutex};
use std::path::PathBuf;

use config::Config;
use users::UserManager;
use logger::Logger;
use ftp_server::FtpServer;
use sftp_server::SftpServer;
use service::ServiceManager;

pub struct AppState {
    pub config: Arc<Mutex<Config>>,
    pub user_manager: Arc<Mutex<UserManager>>,
    pub logger: Arc<Mutex<Logger>>,
    pub ftp_server: Arc<Mutex<Option<FtpServer>>>,
    pub sftp_server: Arc<Mutex<Option<SftpServer>>>,
    pub service_manager: ServiceManager,
    pub config_path: PathBuf,
    pub users_path: PathBuf,
}

impl AppState {
    pub fn new() -> Self {
        let config_path = Config::get_config_path();
        let users_path = Config::get_users_path();
        
        let config = Config::load(&config_path).unwrap_or_else(|_| Config::default());
        let user_manager = UserManager::load(&users_path).unwrap_or_else(|_| UserManager::new());
        
        let mut logger = Logger::new(
            &config.logging.log_dir,
            config.logging.max_log_size,
            config.logging.max_log_files,
        );
        
        let _ = logger.init();
        
        AppState {
            config: Arc::new(Mutex::new(config)),
            user_manager: Arc::new(Mutex::new(user_manager)),
            logger: Arc::new(Mutex::new(logger)),
            ftp_server: Arc::new(Mutex::new(None)),
            sftp_server: Arc::new(Mutex::new(None)),
            service_manager: ServiceManager::new(),
            config_path,
            users_path,
        }
    }
    
    pub fn save_config(&self) -> anyhow::Result<()> {
        let config = self.config.lock().unwrap();
        config.save(&self.config_path)?;
        Ok(())
    }
    
    pub fn save_users(&self) -> anyhow::Result<()> {
        let users = self.user_manager.lock().unwrap();
        users.save(&self.users_path)?;
        Ok(())
    }
    
    pub fn start_ftp(&self) -> anyhow::Result<()> {
        let config = Arc::clone(&self.config);
        let user_manager = Arc::clone(&self.user_manager);
        let logger = Arc::clone(&self.logger);
        
        let server = FtpServer::new(config, user_manager, logger);
        server.start()?;
        
        let mut ftp_server = self.ftp_server.lock().unwrap();
        *ftp_server = Some(server);
        
        self.logger.lock().unwrap().info("FTP", "FTP server started");
        Ok(())
    }
    
    pub fn stop_ftp(&self) {
        let mut ftp_server = self.ftp_server.lock().unwrap();
        if let Some(server) = ftp_server.take() {
            server.stop();
            self.logger.lock().unwrap().info("FTP", "FTP server stopped");
        }
    }
    
    pub fn is_ftp_running(&self) -> bool {
        let ftp_server = self.ftp_server.lock().unwrap();
        ftp_server.as_ref().is_some_and(|s| s.is_running())
    }
    
    pub fn start_sftp(&self) -> anyhow::Result<()> {
        let config = Arc::clone(&self.config);
        let user_manager = Arc::clone(&self.user_manager);
        let logger = Arc::clone(&self.logger);
        
        let server = SftpServer::new(config, user_manager, logger);
        server.start()?;
        
        let mut sftp_server = self.sftp_server.lock().unwrap();
        *sftp_server = Some(server);
        
        self.logger.lock().unwrap().info("SFTP", "SFTP server started");
        Ok(())
    }
    
    pub fn stop_sftp(&self) {
        let mut sftp_server = self.sftp_server.lock().unwrap();
        if let Some(server) = sftp_server.take() {
            server.stop();
            self.logger.lock().unwrap().info("SFTP", "SFTP server stopped");
        }
    }
    
    pub fn is_sftp_running(&self) -> bool {
        let sftp_server = self.sftp_server.lock().unwrap();
        sftp_server.as_ref().is_some_and(|s| s.is_running())
    }
    
    pub fn start_all(&self) -> anyhow::Result<()> {
        let config = self.config.lock().unwrap();
        let ftp_enabled = config.ftp.enabled;
        let sftp_enabled = config.sftp.enabled;
        drop(config);
        
        if ftp_enabled {
            self.start_ftp()?;
        }
        if sftp_enabled {
            self.start_sftp()?;
        }
        
        Ok(())
    }
    
    pub fn stop_all(&self) {
        self.stop_ftp();
        self.stop_sftp();
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
