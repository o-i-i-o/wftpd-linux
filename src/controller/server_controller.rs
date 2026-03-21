use anyhow::Result;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use crate::core::config::Config;
use crate::core::users::UserManager;
use crate::core::logger::Logger;
use crate::core::file_logger::FileLogger;
use crate::core::server_manager::ServerManager;

#[derive(Debug, Clone)]
pub struct ServerStatus {
    pub ftp_running: bool,
    pub sftp_running: bool,
}

#[derive(Debug, Clone)]
pub struct ServiceResponse {
    pub success: bool,
    pub message: String,
}

pub struct ServerController {
    server_manager: ServerManager,
    config: Arc<StdMutex<Config>>,
    user_manager: Arc<StdMutex<UserManager>>,
    logger: Arc<StdMutex<Logger>>,
    file_logger: Arc<StdMutex<FileLogger>>,
}

impl ServerController {
    pub fn new(
        config: Arc<StdMutex<Config>>,
        user_manager: Arc<StdMutex<UserManager>>,
        logger: Arc<StdMutex<Logger>>,
        file_logger: Arc<StdMutex<FileLogger>>,
    ) -> Self {
        let server_manager = ServerManager::new();
        
        ServerController {
            server_manager,
            config,
            user_manager,
            logger,
            file_logger,
        }
    }
    
    pub fn from_server_manager(
        server_manager: ServerManager,
        config: Arc<StdMutex<Config>>,
        user_manager: Arc<StdMutex<UserManager>>,
        logger: Arc<StdMutex<Logger>>,
        file_logger: Arc<StdMutex<FileLogger>>,
    ) -> Self {
        ServerController {
            server_manager,
            config,
            user_manager,
            logger,
            file_logger,
        }
    }
    
    pub async fn start_ftp(&self) -> Result<ServiceResponse> {
        match self.server_manager.start_ftp(
            Arc::clone(&self.config),
            Arc::clone(&self.user_manager),
            Arc::clone(&self.logger),
            Arc::clone(&self.file_logger),
        ).await {
            Ok(()) => {
                if let Ok(mut log) = self.logger.lock() {
                    let (bind_ip, ftp_port) = {
                        if let Ok(cfg) = self.config.lock() {
                            (cfg.ftp.bind_ip.clone(), cfg.server.ftp_port)
                        } else {
                            ("0.0.0.0".to_string(), 21)
                        }
                    };
                    log.info("FTP", &format!("FTP服务已启动，监听 {}:{}", bind_ip, ftp_port));
                }
                Ok(ServiceResponse {
                    success: true,
                    message: "FTP 服务器启动成功".to_string(),
                })
            }
            Err(e) => {
                if let Ok(mut log) = self.logger.lock() {
                    log.error("FTP", &format!("FTP服务器启动失败: {}", e));
                }
                Ok(ServiceResponse {
                    success: false,
                    message: format!("FTP 服务器启动失败: {}", e),
                })
            }
        }
    }
    
    pub async fn stop_ftp(&self) -> Result<ServiceResponse> {
        self.server_manager.stop_ftp(&self.logger).await;
        
        if let Ok(mut log) = self.logger.lock() {
            log.info("FTP", "FTP服务已停止");
        }
        
        Ok(ServiceResponse {
            success: true,
            message: "FTP 服务器停止成功".to_string(),
        })
    }
    
    pub async fn restart_ftp(&self) -> Result<ServiceResponse> {
        self.server_manager.stop_ftp(&self.logger).await;
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        self.start_ftp().await
    }
    
    pub async fn start_sftp(&self) -> Result<ServiceResponse> {
        match self.server_manager.start_sftp(
            Arc::clone(&self.config),
            Arc::clone(&self.user_manager),
            Arc::clone(&self.logger),
            Arc::clone(&self.file_logger),
        ).await {
            Ok(()) => {
                if let Ok(mut log) = self.logger.lock() {
                    let (bind_ip, sftp_port) = {
                        if let Ok(cfg) = self.config.lock() {
                            (cfg.sftp.bind_ip.clone(), cfg.server.sftp_port)
                        } else {
                            ("0.0.0.0".to_string(), 22)
                        }
                    };
                    log.info("SFTP", &format!("SFTP服务已启动，监听 {}:{}", bind_ip, sftp_port));
                }
                Ok(ServiceResponse {
                    success: true,
                    message: "SFTP 服务器启动成功".to_string(),
                })
            }
            Err(e) => {
                if let Ok(mut log) = self.logger.lock() {
                    log.error("SFTP", &format!("SFTP服务器启动失败: {}", e));
                }
                Ok(ServiceResponse {
                    success: false,
                    message: format!("SFTP 服务器启动失败: {}", e),
                })
            }
        }
    }
    
    pub async fn stop_sftp(&self) -> Result<ServiceResponse> {
        self.server_manager.stop_sftp(&self.logger).await;
        
        if let Ok(mut log) = self.logger.lock() {
            log.info("SFTP", "SFTP服务已停止");
        }
        
        Ok(ServiceResponse {
            success: true,
            message: "SFTP 服务器停止成功".to_string(),
        })
    }
    
    pub async fn restart_sftp(&self) -> Result<ServiceResponse> {
        self.server_manager.stop_sftp(&self.logger).await;
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        self.start_sftp().await
    }
    
    pub fn get_status(&self) -> ServerStatus {
        ServerStatus {
            ftp_running: self.server_manager.is_ftp_running(),
            sftp_running: self.server_manager.is_sftp_running(),
        }
    }
    
    pub async fn stop_all(&self) {
        self.server_manager.stop_ftp(&self.logger).await;
        self.server_manager.stop_sftp(&self.logger).await;
    }
}
