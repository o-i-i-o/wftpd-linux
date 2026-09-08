//! 后端服务状态：持有配置、用户、日志与 FTP/SFTP 服务实例。

use std::sync::{Arc, Mutex as StdMutex};

use wftpd_common::tracing_logger::LogBuffer;
use wftpd_common::{Config, FileLogger, UserManager};
use wftpd_ftp::FtpServer;
use wftpd_sftp::SftpServer;

pub struct BackendState {
    pub config: Arc<StdMutex<Config>>,
    pub user_manager: Arc<StdMutex<UserManager>>,
    pub file_logger: Arc<StdMutex<FileLogger>>,
    pub log_buffer: LogBuffer,
    ftp_server: StdMutex<Option<FtpServer>>,
    sftp_server: StdMutex<Option<SftpServer>>,
}

impl BackendState {
    pub fn new() -> anyhow::Result<Self> {
        let config_path = Config::get_config_path();
        let config = Arc::new(StdMutex::new(Config::load(&config_path)?));

        let users_path = Config::get_users_path();
        let user_manager = Arc::new(StdMutex::new(UserManager::load(&users_path)?));

        let (log_dir, log_level, max_log_files, enable_json) = {
            let cfg = config.lock().unwrap();
            (
                cfg.logging.log_dir.clone(),
                cfg.logging.log_level.clone(),
                cfg.logging.max_log_files,
                cfg.logging.enable_json,
            )
        };

        let log_buffer =
            wftpd_common::init_tracing(&log_dir, &log_level, max_log_files, enable_json)?;

        let file_logger = Arc::new(StdMutex::new(FileLogger::new(&log_dir, 10 * 1024 * 1024)));

        Ok(BackendState {
            config,
            user_manager,
            file_logger,
            log_buffer,
            ftp_server: StdMutex::new(None),
            sftp_server: StdMutex::new(None),
        })
    }

    /// 启动 FTP 服务（已运行则直接返回）
    pub async fn start_ftp(&self) -> anyhow::Result<()> {
        {
            let server = self.ftp_server.lock().unwrap();
            if server.is_some() {
                return Ok(());
            }
        }

        let server = FtpServer::new(
            Arc::clone(&self.config),
            Arc::clone(&self.user_manager),
            Arc::clone(&self.file_logger),
        );
        server.start().await?;

        let mut slot = self.ftp_server.lock().unwrap();
        *slot = Some(server);
        Ok(())
    }

    /// 启动 SFTP 服务（已运行则直接返回）
    pub async fn start_sftp(&self) -> anyhow::Result<()> {
        {
            let server = self.sftp_server.lock().unwrap();
            if server.is_some() {
                return Ok(());
            }
        }

        let server = SftpServer::new(
            Arc::clone(&self.config),
            Arc::clone(&self.user_manager),
            Arc::clone(&self.file_logger),
        );
        server.start().await?;

        let mut slot = self.sftp_server.lock().unwrap();
        *slot = Some(server);
        Ok(())
    }

    pub async fn stop_ftp(&self) {
        let server = self.ftp_server.lock().unwrap().take();
        if let Some(server) = server {
            server.stop().await;
        }
    }

    pub async fn stop_sftp(&self) {
        let server = self.sftp_server.lock().unwrap().take();
        if let Some(server) = server {
            let _ = server.stop().await;
        }
    }

    pub async fn restart_ftp(&self) -> anyhow::Result<()> {
        self.stop_ftp().await;
        self.start_ftp().await
    }

    pub async fn restart_sftp(&self) -> anyhow::Result<()> {
        self.stop_sftp().await;
        self.start_sftp().await
    }

    pub async fn stop_all(&self) {
        self.stop_ftp().await;
        self.stop_sftp().await;
    }

    pub fn is_ftp_running(&self) -> bool {
        self.ftp_server
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(wftpd_ftp::FtpServer::is_running)
    }

    pub fn is_sftp_running(&self) -> bool {
        self.sftp_server
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(wftpd_sftp::SftpServer::is_running)
    }

    /// 应用新的启用状态：启动"配置启用但未运行"的服务，停止"配置禁用但仍在运行"的服务。
    /// 其他变更（端口、证书等）需要显式重启对应服务。
    pub async fn apply_enabled_flags(&self) {
        let (ftp_enabled, sftp_enabled) = {
            let cfg = self.config.lock().unwrap();
            (cfg.ftp.enabled, cfg.sftp.enabled)
        };

        if ftp_enabled && !self.is_ftp_running() {
            if let Err(e) = self.start_ftp().await {
                tracing::error!("Failed to start FTP server: {}", e);
            }
        } else if !ftp_enabled && self.is_ftp_running() {
            self.stop_ftp().await;
        }

        if sftp_enabled && !self.is_sftp_running() {
            if let Err(e) = self.start_sftp().await {
                tracing::error!("Failed to start SFTP server: {}", e);
            }
        } else if !sftp_enabled && self.is_sftp_running() {
            self.stop_sftp().await;
        }
    }
}
