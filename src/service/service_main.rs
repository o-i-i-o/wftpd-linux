use anyhow::Result;
use std::sync::{Arc, Mutex};

use crate::core::config::Config;
use crate::core::users::UserManager;
use crate::core::logger::Logger;
use crate::core::file_logger::FileLogger;
use crate::core::server_manager::ServerManager;

pub fn run_service() -> Result<()> {
    let config_path = Config::get_config_path();
    let config = Arc::new(Mutex::new(Config::load(&config_path)?));
    
    let users_path = Config::get_users_path();
    let user_manager = Arc::new(Mutex::new(UserManager::load(&users_path)?));
    
    let log_dir = config.lock().unwrap().logging.log_dir.clone();
    let logger = Arc::new(Mutex::new(Logger::new(&log_dir, 10 * 1024 * 1024, 10)));
    
    let file_logger = Arc::new(Mutex::new(FileLogger::new(&log_dir, 10 * 1024 * 1024)));
    
    let server_manager = ServerManager::new();
    
    {
        let mut log = logger.lock().unwrap();
        log.info("SERVICE", "WFTPG service starting");
    }
    
    let cfg = config.lock().unwrap();
    if cfg.ftp.enabled {
        drop(cfg);
        if let Err(e) = server_manager.start_ftp(
            Arc::clone(&config),
            Arc::clone(&user_manager),
            Arc::clone(&logger),
            Arc::clone(&file_logger),
        ) {
            let mut log = logger.lock().unwrap();
            log.error("SERVICE", &format!("Failed to start FTP server: {}", e));
        }
    }
    
    let cfg = config.lock().unwrap();
    if cfg.sftp.enabled {
        drop(cfg);
        if let Err(e) = server_manager.start_sftp(
            Arc::clone(&config),
            Arc::clone(&user_manager),
            Arc::clone(&logger),
            Arc::clone(&file_logger),
        ) {
            let mut log = logger.lock().unwrap();
            log.error("SERVICE", &format!("Failed to start SFTP server: {}", e));
        }
    }
    
    {
        let mut log = logger.lock().unwrap();
        log.info("SERVICE", "WFTPG service started successfully");
    }
    
    tokio::runtime::Runtime::new()?
        .block_on(async {
            tokio::signal::ctrl_c().await?;
            Ok::<(), anyhow::Error>(())
        })?;
    
    {
        let mut log = logger.lock().unwrap();
        log.info("SERVICE", "WFTPG service shutting down");
    }
    
    server_manager.stop_ftp(&logger);
    server_manager.stop_sftp(&logger);
    
    {
        let mut log = logger.lock().unwrap();
        log.info("SERVICE", "WFTPG service stopped");
    }
    
    Ok(())
}
