pub mod core;
pub mod server;
pub mod service;
pub mod communication;
pub mod ui;

use std::sync::{Arc, Mutex};

use crate::core::config::Config;
use crate::core::file_logger::FileLogger;
use crate::core::logger::Logger;
use crate::core::users::UserManager;
use crate::core::server_manager::ServerManager;
use crate::service::ServiceManager;
use crate::core::tracing_logger::{init_tracing, init_simple};

pub struct AppState {
    pub config: Arc<Mutex<Config>>,
    pub user_manager: Arc<Mutex<UserManager>>,
    pub server_manager: ServerManager,
    pub service_manager: ServiceManager,
    pub logger: Arc<Mutex<Logger>>,
    pub file_logger: Arc<Mutex<FileLogger>>,
}

impl AppState {
    /// 创建 AppState（服务端使用，会初始化文件日志）
    pub fn new() -> anyhow::Result<Self> {
        Self::new_with_logging(true)
    }
    
    /// 创建 AppState（前端使用，只使用控制台日志，不写入文件）
    pub fn new_for_gui() -> anyhow::Result<Self> {
        Self::new_with_logging(false)
    }
    
    fn new_with_logging(enable_file_log: bool) -> anyhow::Result<Self> {
        let config_path = Config::get_config_path();
        let config = Arc::new(Mutex::new(Config::load(&config_path)?));
        
        let users_path = Config::get_users_path();
        let user_manager = Arc::new(Mutex::new(UserManager::load(&users_path)?));
        
        let (log_dir, log_level, max_log_size, max_log_files, enable_json) = {
            let cfg = config.lock().unwrap();
            (
                cfg.logging.log_dir.clone(),
                cfg.logging.log_level.clone(),
                cfg.logging.max_log_size,
                cfg.logging.max_log_files,
                cfg.logging.enable_json,
            )
        };
        
        if enable_file_log {
            init_tracing(&log_dir, &log_level, max_log_files, enable_json)?;
        } else {
            // 前端只使用简单的控制台日志，不写入文件
            init_simple()?;
        }
        
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
    
    pub async fn stop_all(&self) {
        self.server_manager.stop_ftp().await;
        self.server_manager.stop_sftp().await;
    }
}
