use anyhow::Result;
use std::sync::Arc;

use crate::core::config::Config;
use crate::core::users::UserManager;
use crate::core::file_logger::FileLogger;
use crate::core::logger::Logger;
use crate::core::server_manager::ServerManager;
use crate::core::tracing_logger::init_tracing;
use crate::communication::IpcServer;
use tracing::{info, error};

pub fn run_service() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async {
        run_service_async().await
    })
}

async fn run_service_async() -> Result<()> {
    let config_path = Config::get_config_path();
    let config = Arc::new(std::sync::Mutex::new(Config::load(&config_path)?));
    
    {
        let cfg = config.lock().unwrap();
        if let Err(e) = cfg.validate() {
            eprintln!("Configuration validation failed: {}", e);
            return Err(e);
        }
    }
    
    let users_path = Config::get_users_path();
    let user_manager = Arc::new(std::sync::Mutex::new(UserManager::load(&users_path)?));
    
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
    
    init_tracing(&log_dir, &log_level, max_log_files, enable_json)?;
    
    let logger = Arc::new(std::sync::Mutex::new(Logger::new(&log_dir, max_log_size, max_log_files)));
    let server_manager = Arc::new(ServerManager::new());
    
    // 先创建临时的 FileLogger 和 IPC 服务器来获取 log_sender
    let temp_file_logger = Arc::new(std::sync::Mutex::new(FileLogger::new(&log_dir, max_log_size)));
    
    info!("WFTPG service starting");
    
    // 创建临时 IPC 服务器获取 log_sender
    let temp_ipc = IpcServer::new(
        Arc::clone(&config),
        Arc::clone(&user_manager),
        Arc::clone(&server_manager),
        Arc::clone(&logger),
        temp_file_logger,
    );
    
    // 从 IPC 服务器获取 log_sender 并创建真正的 FileLogger
    let log_sender = temp_ipc.get_log_sender();
    let file_logger = Arc::new(std::sync::Mutex::new(FileLogger::with_log_sender(log_sender)));
    
    // 创建正式的 IPC 服务器
    let ipc_server = IpcServer::new(
        Arc::clone(&config),
        Arc::clone(&user_manager),
        Arc::clone(&server_manager),
        Arc::clone(&logger),
        Arc::clone(&file_logger),
    );
    
    let (ftp_enabled, sftp_enabled) = {
        let cfg = config.lock().unwrap();
        (cfg.ftp.enabled, cfg.sftp.enabled)
    };
    
    if ftp_enabled
        && let Err(e) = server_manager.start_ftp(
            Arc::clone(&config),
            Arc::clone(&user_manager),
            Arc::clone(&file_logger),
        ).await {
            error!("Failed to start FTP server: {}", e);
        }
    
    if sftp_enabled
        && let Err(e) = server_manager.start_sftp(
            Arc::clone(&config),
            Arc::clone(&user_manager),
            Arc::clone(&file_logger),
        ).await {
            error!("Failed to start SFTP server: {}", e);
        }
    
    info!("WFTPG service started successfully");
    
    let ipc_task = tokio::spawn(async move {
        if let Err(e) = ipc_server.run().await {
            error!("IPC server error: {}", e);
        }
    });
    
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            info!("Received shutdown signal");
        }
        _ = ipc_task => {
            info!("IPC server stopped");
        }
    }
    
    info!("WFTPG service shutting down");
    
    Ok(())
}
