pub mod core;
pub mod server;
pub mod communication;
pub mod ui;
pub mod service;

use std::sync::{Arc, Mutex};

pub use crate::core::config::Config;
pub use crate::core::logger::Logger;
pub use crate::core::file_logger::FileLogger;
pub use crate::core::users::{User, UserManager, Permissions};
pub use crate::core::server_manager::ServerManager;
pub use crate::server::ftp::FtpServer;
pub use crate::server::sftp::SftpServer;
pub use crate::service::ServiceManager;

pub struct AppState {
    pub config: Arc<Mutex<Config>>,
    pub user_manager: Arc<Mutex<UserManager>>,
    pub server_manager: ServerManager,
    pub service_manager: ServiceManager,
    pub logger: Arc<Mutex<Logger>>,
    pub file_logger: Arc<Mutex<FileLogger>>,
}

impl AppState {
    pub fn new() -> anyhow::Result<Self> {
        let config_path = Config::get_config_path();
        let config = Arc::new(Mutex::new(Config::load(&config_path)?));
        
        let users_path = Config::get_users_path();
        let user_manager = Arc::new(Mutex::new(UserManager::load(&users_path)?));
        
        let (log_dir, max_log_size, max_log_files) = {
            let cfg = config.lock().unwrap();
            (cfg.logging.log_dir.clone(), cfg.logging.max_log_size, cfg.logging.max_log_files)
        };
        
        let logger = Arc::new(Mutex::new(Logger::new(&log_dir, max_log_size, max_log_files)));
        let file_logger = Arc::new(Mutex::new(FileLogger::new(&log_dir, max_log_size)));
        
        let server_manager = ServerManager::new();
        let service_manager = ServiceManager::new();
        
        Ok(AppState {
            config,
            user_manager,
            server_manager,
            service_manager,
            logger,
            file_logger,
        })
    }
    
    pub fn stop_all(&self) {
        self.server_manager.stop_ftp(&self.logger);
        self.server_manager.stop_sftp(&self.logger);
    }
}
