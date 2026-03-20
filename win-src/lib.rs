//! WFTPG - SFTP/FTP Server Library
//! 
//! This library provides the core functionality for the WFTPG SFTP/FTP server.

pub mod core;
pub mod gui_egui;

use std::sync::{Arc, Mutex};
use std::path::PathBuf;

use core::config::Config;
use core::users::UserManager;
use core::logger::Logger;
use core::file_logger::FileLogger;
use core::server_manager::ServerManager;

pub struct AppState {
    pub config: Arc<Mutex<Config>>,
    pub user_manager: Arc<Mutex<UserManager>>,
    pub logger: Arc<Mutex<Logger>>,
    pub file_logger: Arc<Mutex<FileLogger>>,
    server_manager: ServerManager,
    pub config_path: PathBuf,
    pub users_path: PathBuf,
}

impl AppState {
    pub fn new() -> anyhow::Result<Self> {
        let config_path = Config::get_config_path();
        let users_path = Config::get_users_path();
        
        let config = Config::load(&config_path)?;
        let user_manager = UserManager::load(&users_path)?;
        
        let logger = Logger::new(
            &config.logging.log_dir,
            config.logging.max_log_size,
            config.logging.max_log_files,
        );
        
        let file_logger = FileLogger::new(
            &config.logging.log_dir,
            config.logging.max_log_size,
        );
        
        Ok(AppState {
            config: Arc::new(Mutex::new(config)),
            user_manager: Arc::new(Mutex::new(user_manager)),
            logger: Arc::new(Mutex::new(logger)),
            file_logger: Arc::new(Mutex::new(file_logger)),
            server_manager: ServerManager::new(),
            config_path,
            users_path,
        })
    }
    
    pub fn save_config(&self) -> anyhow::Result<()> {
        let config = self.config.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        config.save(&self.config_path)?;
        Ok(())
    }
    
    pub fn save_users(&self) -> anyhow::Result<()> {
        let users = self.user_manager.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        users.save(&self.users_path)?;
        Ok(())
    }
    
    pub fn start_ftp(&self) -> anyhow::Result<()> {
        self.server_manager.start_ftp(
            Arc::clone(&self.config),
            Arc::clone(&self.user_manager),
            Arc::clone(&self.logger),
            Arc::clone(&self.file_logger),
        )
    }
    
    pub fn stop_ftp(&self) {
        self.server_manager.stop_ftp(&self.logger);
    }
    
    pub fn is_ftp_running(&self) -> bool {
        self.server_manager.is_ftp_running()
    }
    
    pub fn start_sftp(&self) -> anyhow::Result<()> {
        self.server_manager.start_sftp(
            Arc::clone(&self.config),
            Arc::clone(&self.user_manager),
            Arc::clone(&self.logger),
            Arc::clone(&self.file_logger),
        )
    }
    
    pub fn stop_sftp(&self) {
        self.server_manager.stop_sftp(&self.logger);
    }
    
    pub fn is_sftp_running(&self) -> bool {
        self.server_manager.is_sftp_running()
    }
    
    pub fn start_all(&self) -> anyhow::Result<()> {
        let (ftp_enabled, sftp_enabled) = {
            let config = self.config.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
            (config.ftp.enabled, config.sftp.enabled)
        };
        
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
    
    pub fn reload_config(&self) -> anyhow::Result<()> {
        let config = crate::core::config::Config::load(&self.config_path)?;
        let mut current_config = self.config.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        *current_config = config;
        Ok(())
    }
    
    pub fn reload_users(&self) -> anyhow::Result<()> {
        let users = crate::core::users::UserManager::load(&self.users_path)?;
        let mut current_users = self.user_manager.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        *current_users = users;
        Ok(())
    }

    pub fn get_logs(&self, count: usize) -> Vec<crate::core::ipc::LogEntryDto> {
        if let Ok(logger) = self.logger.try_lock() {
            let entries = logger.get_recent_logs(count);
            entries.into_iter().map(|e| crate::core::ipc::LogEntryDto {
                timestamp: e.timestamp.format("%Y-%m-%d %H:%M:%S").to_string(),
                level: e.level.to_string(),
                source: e.source,
                message: e.message,
                client_ip: e.client_ip,
                username: e.username,
                action: e.action,
            }).collect()
        } else {
            Vec::new()
        }
    }

    pub fn get_file_logs(&self, count: usize) -> Vec<crate::core::ipc::FileLogEntryDto> {
        if let Ok(file_logger) = self.file_logger.try_lock() {
            let entries = file_logger.get_recent_logs(count);
            entries.into_iter().map(|e| crate::core::ipc::FileLogEntryDto {
                timestamp: e.timestamp.format("%Y-%m-%d %H:%M:%S").to_string(),
                username: e.username,
                client_ip: e.client_ip,
                operation: e.operation,
                file_path: e.file_path,
                file_size: e.file_size,
                protocol: e.protocol,
                success: e.success,
                message: e.message,
            }).collect()
        } else {
            Vec::new()
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new().expect("Failed to create default AppState")
    }
}
