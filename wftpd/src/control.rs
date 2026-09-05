//! gRPC 控制服务实现：服务生命周期、配置/用户管理、日志读取与订阅。
//!
//! 语义约定：
//! - Start/StopService 会同步更新 config 中对应服务的 enabled 标志并落盘，
//!   保证“运行意图”在守护进程重启后保持一致；
//! - SaveConfig / SaveUsers 先校验后落盘，失败时通过 OpReply 返回原因；
//! - 所有文件写入都由后端完成，前端不直接改动配置与用户文件。

use std::io::Write;
use std::sync::Arc;

use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};
use tracing::{error, info};
use wftpd_common::paths;
use wftpd_proto::Control;
use wftpd_proto::wftpd::v1::service_selector::Which;
use wftpd_proto::wftpd::v1::{
    ConfigReply, CreateUserDirectoryRequest, EnsureUserDirectoriesRequest, FileOpLogEntry,
    FileOpLogsReply, GetConfigRequest, GetFileOpLogContentRequest, GetFileOpLogFilesRequest,
    GetInitialStateRequest, GetLogFileContentRequest, GetLogFilesRequest, GetRecentLogsRequest,
    GetStatusRequest, InitialStateReply, LogEvent, LogFileInfo, LogFilesReply, LogsReply, OpReply,
    SaveConfigRequest, SaveLogConfigRequest, SaveUsersRequest, ServiceSelector,
    SetupDirectoryPermissionsRequest, StatusReply, UsersReply, WatchLogsRequest,
    WriteAuditLogRequest,
};

use crate::logs;
use crate::state::BackendState;

pub struct ControlService {
    state: Arc<BackendState>,
}

impl ControlService {
    pub fn new(state: Arc<BackendState>) -> Self {
        ControlService { state }
    }

    fn op_ok(message: &str) -> Response<OpReply> {
        Response::new(OpReply {
            success: true,
            message: message.to_string(),
        })
    }

    fn op_err(message: &str) -> Response<OpReply> {
        Response::new(OpReply {
            success: false,
            message: message.to_string(),
        })
    }
}

fn internal(e: impl std::fmt::Display) -> Status {
    Status::internal(e.to_string())
}

#[tonic::async_trait]
impl Control for ControlService {
    async fn get_status(
        &self,
        _request: Request<GetStatusRequest>,
    ) -> Result<Response<StatusReply>, Status> {
        Ok(Response::new(StatusReply {
            ftp_running: self.state.is_ftp_running(),
            sftp_running: self.state.is_sftp_running(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            config_path: paths::config_path().to_string_lossy().into_owned(),
            users_path: paths::users_path().to_string_lossy().into_owned(),
            socket_path: paths::socket_path().to_string_lossy().into_owned(),
        }))
    }

    async fn start_service(
        &self,
        request: Request<ServiceSelector>,
    ) -> Result<Response<OpReply>, Status> {
        let which = request.into_inner().which();
        let result = match which {
            Which::Ftp => {
                self.set_ftp_enabled(true);
                self.state.start_ftp().await
            }
            Which::Sftp => {
                self.set_sftp_enabled(true);
                self.state.start_sftp().await
            }
            Which::All => {
                self.set_ftp_enabled(true);
                self.set_sftp_enabled(true);
                let ftp = self.state.start_ftp().await;
                let sftp = self.state.start_sftp().await;
                ftp.and(sftp)
            }
        };

        match result {
            Ok(()) => Ok(Self::op_ok("服务已启动")),
            Err(e) => {
                error!("start service failed: {}", e);
                Ok(Self::op_err(&format!("启动失败: {e}")))
            }
        }
    }

    async fn stop_service(
        &self,
        request: Request<ServiceSelector>,
    ) -> Result<Response<OpReply>, Status> {
        match request.into_inner().which() {
            Which::Ftp => {
                self.set_ftp_enabled(false);
                self.state.stop_ftp().await;
            }
            Which::Sftp => {
                self.set_sftp_enabled(false);
                self.state.stop_sftp().await;
            }
            Which::All => {
                self.set_ftp_enabled(false);
                self.set_sftp_enabled(false);
                self.state.stop_all().await;
            }
        }
        Ok(Self::op_ok("服务已停止"))
    }

    async fn restart_service(
        &self,
        request: Request<ServiceSelector>,
    ) -> Result<Response<OpReply>, Status> {
        let result = match request.into_inner().which() {
            Which::Ftp => self.state.restart_ftp().await,
            Which::Sftp => self.state.restart_sftp().await,
            Which::All => {
                let ftp = self.state.restart_ftp().await;
                let sftp = self.state.restart_sftp().await;
                ftp.and(sftp)
            }
        };

        match result {
            Ok(()) => Ok(Self::op_ok("服务已重启")),
            Err(e) => {
                error!("restart service failed: {}", e);
                Ok(Self::op_err(&format!("重启失败: {e}")))
            }
        }
    }

    async fn get_config(
        &self,
        _request: Request<GetConfigRequest>,
    ) -> Result<Response<ConfigReply>, Status> {
        let content =
            std::fs::read_to_string(wftpd_common::Config::get_config_path()).map_err(internal)?;
        Ok(Response::new(ConfigReply { content }))
    }

    async fn save_config(
        &self,
        request: Request<SaveConfigRequest>,
    ) -> Result<Response<ConfigReply>, Status> {
        let content = request.into_inner().content;

        let new_config: wftpd_common::Config = toml::from_str(&content)
            .map_err(|e| Status::invalid_argument(format!("配置解析失败: {e}")))?;

        new_config
            .validate()
            .map_err(|e| Status::invalid_argument(format!("配置校验失败: {e}")))?;

        let config_path = wftpd_common::Config::get_config_path();
        new_config.save(&config_path).map_err(internal)?;

        {
            let mut cfg = self.state.config.lock().unwrap();
            *cfg = new_config;
        }

        // 应用启用标志（端口/证书等变更需要显式重启服务）
        self.state.apply_enabled_flags().await;

        let content = std::fs::read_to_string(&config_path).map_err(internal)?;
        Ok(Response::new(ConfigReply { content }))
    }

    async fn get_users(
        &self,
        _request: Request<wftpd_proto::GetUsersRequest>,
    ) -> Result<Response<UsersReply>, Status> {
        let content =
            std::fs::read_to_string(wftpd_common::Config::get_users_path()).map_err(internal)?;
        Ok(Response::new(UsersReply { content }))
    }

    async fn save_users(
        &self,
        request: Request<SaveUsersRequest>,
    ) -> Result<Response<UsersReply>, Status> {
        let content = request.into_inner().content;

        // 先写入临时文件并用 UserManager 加载校验，避免半成品用户库落盘
        let users_path = wftpd_common::Config::get_users_path();
        let temp_path = users_path.with_extension("json.tmp");
        std::fs::write(&temp_path, &content).map_err(internal)?;
        let validated = wftpd_common::UserManager::load(&temp_path).map_err(|e| {
            let _ = std::fs::remove_file(&temp_path);
            Status::invalid_argument(format!("用户数据校验失败: {e}"))
        })?;
        std::fs::rename(&temp_path, &users_path).map_err(internal)?;

        {
            let mut manager = self.state.user_manager.lock().unwrap();
            *manager = validated;
        }

        Ok(Response::new(UsersReply { content }))
    }

    async fn get_initial_state(
        &self,
        _request: Request<GetInitialStateRequest>,
    ) -> Result<Response<InitialStateReply>, Status> {
        let config =
            std::fs::read_to_string(wftpd_common::Config::get_config_path()).map_err(internal)?;
        let users =
            std::fs::read_to_string(wftpd_common::Config::get_users_path()).map_err(internal)?;

        Ok(Response::new(InitialStateReply {
            config,
            users,
            ftp_running: self.state.is_ftp_running(),
            sftp_running: self.state.is_sftp_running(),
        }))
    }

    async fn ensure_user_directories(
        &self,
        _request: Request<EnsureUserDirectoriesRequest>,
    ) -> Result<Response<OpReply>, Status> {
        let home_dirs: Vec<String> = {
            let manager = self.state.user_manager.lock().unwrap();
            manager
                .list_users()
                .map(|(_, user)| user.home_dir.clone())
                .collect()
        };

        let mut failed = Vec::new();
        for dir in home_dirs {
            let path = std::path::Path::new(&dir);
            if !path.exists()
                && let Err(e) = std::fs::create_dir_all(path)
            {
                failed.push(format!("{dir}: {e}"));
            }
        }

        if failed.is_empty() {
            Ok(Self::op_ok("用户目录已就绪"))
        } else {
            Ok(Self::op_err(&format!(
                "部分目录创建失败: {}",
                failed.join("; ")
            )))
        }
    }

    async fn create_user_directory(
        &self,
        request: Request<CreateUserDirectoryRequest>,
    ) -> Result<Response<OpReply>, Status> {
        let path = request.into_inner().path;
        match std::fs::create_dir_all(&path) {
            Ok(()) => Ok(Self::op_ok("目录已创建")),
            Err(e) => Ok(Self::op_err(&format!("创建目录失败: {e}"))),
        }
    }

    async fn setup_directory_permissions(
        &self,
        request: Request<SetupDirectoryPermissionsRequest>,
    ) -> Result<Response<OpReply>, Status> {
        let path = request.into_inner().path;
        let path = std::path::Path::new(&path);

        if let Err(e) = std::fs::create_dir_all(path) {
            return Ok(Self::op_err(&format!("创建目录失败: {e}")));
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)) {
                return Ok(Self::op_err(&format!("设置权限失败: {e}")));
            }
        }

        Ok(Self::op_ok("目录权限已设置 (0755)"))
    }

    async fn get_recent_logs(
        &self,
        request: Request<GetRecentLogsRequest>,
    ) -> Result<Response<LogsReply>, Status> {
        let count = request.into_inner().count as usize;
        let entries: Vec<wftpd_proto::LogEntry> = self
            .state
            .log_buffer
            .recent(count)
            .into_iter()
            .map(Into::into)
            .collect();
        Ok(Response::new(LogsReply { entries }))
    }

    type WatchLogsStream = std::pin::Pin<Box<ReceiverStream<Result<LogEvent, Status>>>>;

    async fn watch_logs(
        &self,
        request: Request<WatchLogsRequest>,
    ) -> Result<Response<Self::WatchLogsStream>, Status> {
        let tail_count = request.into_inner().tail_count as usize;

        let gui_logging_enabled = {
            let cfg = self.state.config.lock().unwrap();
            cfg.logging.enable_gui_logging
        };

        let (tx, rx) = mpsc::channel::<Result<LogEvent, Status>>(64);

        if gui_logging_enabled {
            let history = self.state.log_buffer.recent(tail_count);
            let mut subscribe_rx = self.state.log_buffer.subscribe();

            tokio::spawn(async move {
                for entry in history {
                    let event = LogEvent {
                        entry: Some(entry.into()),
                    };
                    if tx.send(Ok(event)).await.is_err() {
                        return;
                    }
                }
                loop {
                    match subscribe_rx.recv().await {
                        Ok(entry) => {
                            let event = LogEvent {
                                entry: Some(entry.into()),
                            };
                            if tx.send(Ok(event)).await.is_err() {
                                return;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                    }
                }
            });
        } else {
            info!("WatchLogs requested but gui logging is disabled in config");
        }

        Ok(Response::new(Box::pin(ReceiverStream::new(rx))))
    }

    async fn get_log_files(
        &self,
        _request: Request<GetLogFilesRequest>,
    ) -> Result<Response<LogFilesReply>, Status> {
        let log_dir = {
            let cfg = self.state.config.lock().unwrap();
            cfg.logging.log_dir.clone()
        };
        let files: Vec<LogFileInfo> = logs::list_log_files(&log_dir, "wftpg")
            .into_iter()
            .map(Into::into)
            .collect();
        Ok(Response::new(LogFilesReply { files }))
    }

    async fn get_log_file_content(
        &self,
        request: Request<GetLogFileContentRequest>,
    ) -> Result<Response<LogsReply>, Status> {
        let req = request.into_inner();
        let log_dir = {
            let cfg = self.state.config.lock().unwrap();
            cfg.logging.log_dir.clone()
        };

        logs::ensure_path_in_dir(&req.path, &log_dir)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let entries: Vec<wftpd_proto::LogEntry> =
            logs::read_program_log(&req.path, req.count as usize)
                .into_iter()
                .map(Into::into)
                .collect();
        Ok(Response::new(LogsReply { entries }))
    }

    async fn get_file_op_log_files(
        &self,
        _request: Request<GetFileOpLogFilesRequest>,
    ) -> Result<Response<LogFilesReply>, Status> {
        let log_dir = {
            let cfg = self.state.config.lock().unwrap();
            cfg.logging.log_dir.clone()
        };
        let files: Vec<LogFileInfo> = logs::list_log_files(&log_dir, "file-ops")
            .into_iter()
            .map(Into::into)
            .collect();
        Ok(Response::new(LogFilesReply { files }))
    }

    async fn get_file_op_log_content(
        &self,
        request: Request<GetFileOpLogContentRequest>,
    ) -> Result<Response<FileOpLogsReply>, Status> {
        let req = request.into_inner();
        let count = req.count as usize;

        // "current" 特指内存缓冲中的最新文件操作日志
        if req.path == "current" {
            let entries: Vec<FileOpLogEntry> = {
                let logger = self.state.file_logger.lock().unwrap();
                logger
                    .get_recent_logs(count)
                    .iter()
                    .map(wftpd_common::FileLogEntryJson::from)
                    .map(Into::into)
                    .collect()
            };
            return Ok(Response::new(FileOpLogsReply { entries }));
        }

        let log_dir = {
            let cfg = self.state.config.lock().unwrap();
            cfg.logging.log_dir.clone()
        };
        logs::ensure_path_in_dir(&req.path, &log_dir)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let entries: Vec<FileOpLogEntry> = logs::read_file_op_log(&req.path, count)
            .into_iter()
            .map(Into::into)
            .collect();
        Ok(Response::new(FileOpLogsReply { entries }))
    }

    async fn save_log_config(
        &self,
        request: Request<SaveLogConfigRequest>,
    ) -> Result<Response<OpReply>, Status> {
        let req = request.into_inner();
        let level_changed;
        let log_dir_changed;

        {
            let mut cfg = self.state.config.lock().unwrap();
            log_dir_changed = cfg.logging.log_dir != req.log_dir;
            level_changed = cfg.logging.log_level != req.log_level;
            cfg.logging.log_dir = req.log_dir;
            cfg.logging.log_level = req.log_level.clone();
            cfg.logging.max_log_size = req.max_log_size;
            cfg.logging.max_log_files = req.max_log_files as usize;
            cfg.logging.enable_gui_logging = req.enable_gui_logging;

            cfg.save(&wftpd_common::Config::get_config_path())
                .map_err(internal)?;
        }

        // 日志级别立即生效；日志目录/文件数上限在下次重启后生效
        if level_changed {
            wftpd_common::set_log_level(&req.log_level).map_err(internal)?;
        }

        let mut message = String::from("日志配置已保存");
        if log_dir_changed {
            message.push_str("（日志目录变更将在服务重启后生效）");
        }
        Ok(Self::op_ok(&message))
    }

    async fn write_audit_log(
        &self,
        request: Request<WriteAuditLogRequest>,
    ) -> Result<Response<OpReply>, Status> {
        let req = request.into_inner();

        let audit_path = paths::audit_log_path();
        if let Some(parent) = audit_path.parent() {
            std::fs::create_dir_all(parent).map_err(internal)?;
        }

        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&audit_path)
            .map_err(internal)?;

        let timestamp = chrono::Local::now().to_rfc3339();
        let line = format!(
            "{} | {} | {} | {} | {}\n",
            timestamp, req.user, req.action, req.target, req.details
        );
        file.write_all(line.as_bytes()).map_err(internal)?;

        info!(
            target: "audit",
            username = %req.user,
            action = %req.action,
            "GUI 审计: {} {}",
            req.target,
            req.details
        );

        Ok(Self::op_ok("审计日志已写入"))
    }
}

impl ControlService {
    fn persist_config(&self) {
        let cfg = self.state.config.lock().unwrap();
        let path = wftpd_common::Config::get_config_path();
        if let Err(e) = cfg.save(&path) {
            error!("Failed to persist config: {}", e);
        }
    }

    fn set_ftp_enabled(&self, enabled: bool) {
        {
            let mut cfg = self.state.config.lock().unwrap();
            if cfg.ftp.enabled == enabled {
                return;
            }
            cfg.ftp.enabled = enabled;
        }
        self.persist_config();
    }

    fn set_sftp_enabled(&self, enabled: bool) {
        {
            let mut cfg = self.state.config.lock().unwrap();
            if cfg.sftp.enabled == enabled {
                return;
            }
            cfg.sftp.enabled = enabled;
        }
        self.persist_config();
    }
}
