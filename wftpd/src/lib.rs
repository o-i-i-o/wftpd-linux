pub mod core;
pub mod server;

use std::sync::{Arc, Mutex};

use crate::core::config::Config;
use crate::core::file_logger::FileLogger;
use crate::core::users::UserManager;
use crate::core::tracing_logger::init_tracing;
use crate::server::ftp::FtpServer;
use crate::server::sftp::SftpServer;

/// 服务端应用状态，管理 FTP 和 SFTP 服务实例
pub struct AppState {
    pub config: Arc<Mutex<Config>>,
    pub user_manager: Arc<Mutex<UserManager>>,
    pub file_logger: Arc<Mutex<FileLogger>>,
    pub ftp_server: Option<FtpServer>,
    pub sftp_server: Option<SftpServer>,
}

impl AppState {
    /// 创建 AppState（服务端使用，会初始化文件日志）
    pub fn new() -> anyhow::Result<Self> {
        let config_path = Config::get_config_path();
        let config = Arc::new(Mutex::new(Config::load(&config_path)?));
        
        let users_path = Config::get_users_path();
        let user_manager = Arc::new(Mutex::new(UserManager::load(&users_path)?));
        
        let (log_dir, log_level, max_log_files, enable_json) = {
            let cfg = config.lock().unwrap();
            (
                cfg.logging.log_dir.clone(),
                cfg.logging.log_level.clone(),
                cfg.logging.max_log_files,
                cfg.logging.enable_json,
            )
        };
        
        init_tracing(&log_dir, &log_level, max_log_files, enable_json)?;
        
        let file_logger = Arc::new(Mutex::new(FileLogger::new(&log_dir, 10 * 1024 * 1024)));
        
        Ok(AppState {
            config,
            user_manager,
            file_logger,
            ftp_server: None,
            sftp_server: None,
        })
    }
    
    /// 启动 FTP 服务
    pub async fn start_ftp(&mut self) -> anyhow::Result<()> {
        if self.ftp_server.is_some() {
            return Ok(()); // 已经运行
        }
        
        let server = FtpServer::new(
            Arc::clone(&self.config),
            Arc::clone(&self.user_manager),
            Arc::clone(&self.file_logger),
        );
        server.start().await?;
        self.ftp_server = Some(server);
        Ok(())
    }
    
    /// 启动 SFTP 服务
    pub async fn start_sftp(&mut self) -> anyhow::Result<()> {
        if self.sftp_server.is_some() {
            return Ok(()); // 已经运行
        }
        
        let server = SftpServer::new(
            Arc::clone(&self.config),
            Arc::clone(&self.user_manager),
            Arc::clone(&self.file_logger),
        );
        server.start().await?;
        self.sftp_server = Some(server);
        Ok(())
    }
    
    /// 停止 FTP 服务
    pub async fn stop_ftp(&mut self) {
        if let Some(server) = self.ftp_server.take() {
            let _ = server.stop().await;
        }
    }
    
    /// 停止 SFTP 服务
    pub async fn stop_sftp(&mut self) {
        if let Some(server) = self.sftp_server.take() {
            let _ = server.stop().await;
        }
    }
    
    /// 停止所有服务
    pub async fn stop_all(&mut self) {
        self.stop_ftp().await;
        self.stop_sftp().await;
    }
}
