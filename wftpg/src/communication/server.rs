use anyhow::Result;
use std::ffi::CString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::broadcast;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing::{info, debug, warn, error};

use super::protocol::*;
use crate::core::server_manager::ServerManager;
use crate::core::config::Config;
use crate::core::users::UserManager;
use crate::core::logger::Logger;
use crate::core::file_logger::FileLogger;

pub struct IpcServer {
    config: Arc<std::sync::Mutex<Config>>,
    user_manager: Arc<std::sync::Mutex<UserManager>>,
    server_manager: Arc<ServerManager>,
    logger: Arc<std::sync::Mutex<Logger>>,
    file_logger: Arc<std::sync::Mutex<FileLogger>>,
    log_sender: broadcast::Sender<LogEntryJson>,
}

impl IpcServer {
    pub fn new(
        config: Arc<std::sync::Mutex<Config>>,
        user_manager: Arc<std::sync::Mutex<UserManager>>,
        server_manager: Arc<ServerManager>,
        logger: Arc<std::sync::Mutex<Logger>>,
        file_logger: Arc<std::sync::Mutex<FileLogger>>,
    ) -> Self {
        let (log_sender, _) = broadcast::channel(256);
        
        Self {
            config,
            user_manager,
            server_manager,
            logger,
            file_logger,
            log_sender,
        }
    }

    pub async fn run(&self) -> Result<()> {
        let socket_path = Path::new(SOCKET_PATH);
        
        if let Some(parent) = socket_path.parent() {
            fs::create_dir_all(parent)?;
            fs::set_permissions(parent, fs::Permissions::from_mode(0o770))?;
        }
        
        if socket_path.exists() {
            fs::remove_file(socket_path)?;
        }
        
        let listener = UnixListener::bind(socket_path)?;
        fs::set_permissions(socket_path, fs::Permissions::from_mode(0o666))?;
        
        info!(socket_path = %SOCKET_PATH, "IPC server listening");
        
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let server = self.clone_handler();
                    tokio::spawn(async move {
                        if let Err(e) = server.handle_connection(stream).await {
                            error!(error = %e, "IPC connection error");
                        }
                    });
                }
                Err(e) => {
                    error!(error = %e, "Failed to accept IPC connection");
                }
            }
        }
    }

    fn clone_handler(&self) -> Self {
        Self {
            config: Arc::clone(&self.config),
            user_manager: Arc::clone(&self.user_manager),
            server_manager: self.server_manager.clone(),
            logger: Arc::clone(&self.logger),
            file_logger: Arc::clone(&self.file_logger),
            log_sender: self.log_sender.clone(),
        }
    }

    async fn handle_connection(&self, stream: UnixStream) -> Result<()> {
        let peer_cred = stream.peer_cred()?;
        let uid = peer_cred.uid();
        
        if let Err(e) = Self::check_permission(uid) {
            let error_response = IpcResponse::error(0, &format!("Permission denied: {}", e));
            let error_json = serde_json::to_string(&error_response)?;
            let mut stream = stream;
            let _ = stream.write_all(format!("{}\n", error_json).as_bytes()).await;
            return Err(e);
        }
        
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);
        let mut line = String::new();
        
        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) => break,
                Ok(_) => {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    
                    match serde_json::from_str::<IpcRequest>(line) {
                        Ok(request) => {
                            let response = self.handle_request(request).await;
                            let response_json = serde_json::to_string(&response)?;
                            writer.write_all(format!("{}\n", response_json).as_bytes()).await?;
                        }
                        Err(e) => {
                            let error_response = IpcResponse::error(0, &format!("Invalid request: {}", e));
                            let error_json = serde_json::to_string(&error_response)?;
                            writer.write_all(format!("{}\n", error_json).as_bytes()).await?;
                        }
                    }
                }
                Err(e) => {
                    error!(error = %e, "Failed to read from IPC client");
                    break;
                }
            }
        }
        
        Ok(())
    }

    fn check_permission(uid: u32) -> Result<()> {
        if uid == 0 {
            return Ok(());
        }
        
        let user = nix::unistd::User::from_uid(nix::unistd::Uid::from_raw(uid))?
            .ok_or_else(|| anyhow::anyhow!("User not found"))?;
        
        let wftpg_group = nix::unistd::Group::from_name(WFTPG_GROUP_NAME)?;
        
        if let Some(group) = wftpg_group {
            let user_name_c = CString::new(user.name.as_bytes())?;
            let user_groups = nix::unistd::getgrouplist(&user_name_c, user.gid)?;
            
            for g in user_groups {
                if g == group.gid {
                    return Ok(());
                }
            }
        }
        
        Err(anyhow::anyhow!("Need root or wftpg group permission"))
    }

    async fn handle_request(&self, request: IpcRequest) -> IpcResponse {
        match request.command {
            IpcCommand::ReloadConfig => self.handle_reload_config(request.id).await,
            IpcCommand::ReloadUsers => self.handle_reload_users(request.id).await,
            IpcCommand::GetConfig => self.handle_get_config(request.id).await,
            IpcCommand::SaveConfig { content } => self.handle_save_config(request.id, content).await,
            IpcCommand::GetUsers => self.handle_get_users(request.id).await,
            IpcCommand::SaveUsers { content } => self.handle_save_users(request.id, content).await,
            IpcCommand::RestartService => self.handle_restart_service(request.id).await,
            IpcCommand::GetStatus => self.handle_get_status(request.id).await,
            IpcCommand::GetLogs { count } => self.handle_get_logs(request.id, count).await,
            IpcCommand::SubscribeLogs => IpcResponse::error(request.id, "Use separate log subscription"),
            IpcCommand::UnsubscribeLogs => IpcResponse::success(request.id, "Unsubscribed"),
            IpcCommand::WriteAuditLog { user, action, target, details } => {
                self.handle_write_audit_log(request.id, &user, &action, &target, &details).await
            }
            IpcCommand::ConfigExists => self.handle_config_exists(request.id).await,
            IpcCommand::UsersExists => self.handle_users_exists(request.id).await,
            IpcCommand::GetInitialState => self.handle_get_initial_state(request.id).await,
            IpcCommand::EnsureUserDirectories => self.handle_ensure_user_directories(request.id).await,
            IpcCommand::GetLogFiles => self.handle_get_log_files(request.id).await,
            IpcCommand::GetLogFileContent { path, count } => self.handle_get_log_file_content(request.id, &path, count).await,
            IpcCommand::GetFileLogFiles => self.handle_get_file_log_files(request.id).await,
            IpcCommand::GetFileLogFileContent { path, count } => self.handle_get_file_log_file_content(request.id, &path, count).await,
            IpcCommand::SaveLogConfig { log_dir, log_level, max_log_size, max_log_files, _log_to_file, enable_gui_logging } => {
                self.handle_save_log_config(request.id, &log_dir, &log_level, max_log_size, max_log_files, _log_to_file, enable_gui_logging).await
            }
            IpcCommand::SetupDirectoryPermissions { path } => self.handle_setup_directory_permissions(request.id, &path).await,
            IpcCommand::CreateUserDirectory { path } => self.handle_create_user_directory(request.id, &path).await,
        }
    }

    async fn handle_reload_config(&self, id: u64) -> IpcResponse {
        let config_path = Config::get_config_path();
        match Config::load(&config_path) {
            Ok(new_config) => {
                let mut config = self.config.lock().unwrap();
                *config = new_config;
                IpcResponse::success(id, "Configuration reloaded")
            }
            Err(e) => IpcResponse::error(id, &format!("Failed to reload config: {}", e)),
        }
    }

    async fn handle_reload_users(&self, id: u64) -> IpcResponse {
        info!("[IPC] 收到重新加载用户配置请求");
        let users_path = Config::get_users_path();
        debug!(users_path = ?users_path, "[IPC] 重新加载用户配置");
        match UserManager::load(&users_path) {
            Ok(new_users) => {
                let mut users = self.user_manager.lock().unwrap();
                let old_count = users.get_users().len();
                let new_count = new_users.get_users().len();
                *users = new_users;
                info!(old_count = old_count, new_count = new_count, "[IPC] 用户配置重新加载完成");
                IpcResponse::success(id, "Users reloaded")
            }
            Err(e) => {
                error!(error = %e, "[IPC] 重新加载用户配置失败");
                IpcResponse::error(id, &format!("Failed to reload users: {}", e))
            }
        }
    }

    async fn handle_get_config(&self, id: u64) -> IpcResponse {
        match fs::read_to_string(CONFIG_PATH) {
            Ok(content) => IpcResponse::config(id, content),
            Err(e) => IpcResponse::error(id, &format!("Failed to read config: {}", e)),
        }
    }

    async fn handle_save_config(&self, id: u64, content: String) -> IpcResponse {
        if let Some(parent) = Path::new(CONFIG_PATH).parent()
            && let Err(e) = fs::create_dir_all(parent) {
                return IpcResponse::error(id, &format!("Failed to create config directory: {}", e));
            }
        
        match fs::write(CONFIG_PATH, &content) {
            Ok(()) => {
                let config_path = Config::get_config_path();
                match Config::load(&config_path) {
                    Ok(new_config) => {
                        {
                            let mut config = self.config.lock().unwrap();
                            *config = new_config;
                        }
                        match fs::read_to_string(CONFIG_PATH) {
                            Ok(saved_content) => IpcResponse {
                                id,
                                result: IpcResult::ConfigSaved {
                                    message: "Configuration saved".to_string(),
                                    content: saved_content,
                                },
                            },
                            Err(e) => IpcResponse::error(id, &format!("Failed to read saved config: {}", e)),
                        }
                    }
                    Err(e) => IpcResponse::error(id, &format!("Failed to load saved config: {}", e)),
                }
            }
            Err(e) => IpcResponse::error(id, &format!("Failed to save config: {}", e)),
        }
    }

    async fn handle_get_users(&self, id: u64) -> IpcResponse {
        if !Path::new(USERS_PATH).exists() {
            return IpcResponse::users(id, "{}".to_string());
        }
        
        match fs::read_to_string(USERS_PATH) {
            Ok(content) => IpcResponse::users(id, content),
            Err(e) => IpcResponse::error(id, &format!("Failed to read users: {}", e)),
        }
    }

    async fn handle_save_users(&self, id: u64, content: String) -> IpcResponse {
        info!(content_size = content.len(), "[IPC] 收到保存用户配置请求");
        
        if let Some(parent) = Path::new(USERS_PATH).parent()
            && let Err(e) = fs::create_dir_all(parent) {
                error!(error = %e, "[IPC] 创建用户配置目录失败");
                return IpcResponse::error(id, &format!("Failed to create users directory: {}", e));
            }
        
        debug!(users_path = %USERS_PATH, "[IPC] 保存用户配置");
        match fs::write(USERS_PATH, &content) {
            Ok(()) => {
                info!("[IPC] 用户配置已成功写入磁盘");
                let users_path = Config::get_users_path();
                debug!(users_path = ?users_path, "[IPC] 重新加载用户配置");
                match UserManager::load(&users_path) {
                    Ok(new_users) => {
                        {
                            let mut users = self.user_manager.lock().unwrap();
                            let old_count = users.get_users().len();
                            let new_count = new_users.get_users().len();
                            *users = new_users;
                            info!(old_count = old_count, new_count = new_count, "[IPC] 内存中用户配置已更新");
                        }
                        match fs::read_to_string(USERS_PATH) {
                            Ok(saved_content) => {
                                info!("[IPC] 用户配置保存并加载完成");
                                IpcResponse {
                                id,
                                result: IpcResult::UsersSaved {
                                    message: "Users saved".to_string(),
                                    content: saved_content,
                                },
                            }},
                            Err(e) => {
                                error!(error = %e, "[IPC] 读取保存的用户配置失败");
                                IpcResponse::error(id, &format!("Failed to read saved users: {}", e))
                            },
                        }
                    }
                    Err(e) => {
                        error!(error = %e, "[IPC] 加载保存的用户配置失败");
                        IpcResponse::error(id, &format!("Failed to load saved users: {}", e))
                    },
                }
            }
            Err(e) => {
                error!(error = %e, "[IPC] 写入用户配置文件失败");
                IpcResponse::error(id, &format!("Failed to save users: {}", e))
            },
        }
    }

    async fn handle_restart_service(&self, id: u64) -> IpcResponse {
        // 重启服务时根据配置文件的 enabled 设置自动启动 FTP/SFTP
        let (ftp_enabled, sftp_enabled) = {
            let cfg = self.config.lock().unwrap();
            (cfg.ftp.enabled, cfg.sftp.enabled)
        };
        
        info!(
            ftp_enabled = ftp_enabled,
            sftp_enabled = sftp_enabled,
            "[SERVICE] 重启服务，将根据配置启动 FTP/SFTP"
        );
        
        // 注意：服务启停由 systemd 管理，这里只返回成功响应
        // 实际的服务重启通过 systemctl restart wftpd 实现
        IpcResponse::success(id, "Service restart requested - use systemctl restart wftpd")
    }

    async fn handle_get_status(&self, id: u64) -> IpcResponse {
        let ftp_running = self.server_manager.is_ftp_running();
        let sftp_running = self.server_manager.is_sftp_running();
        IpcResponse::status(id, ftp_running, sftp_running)
    }

    async fn handle_get_logs(&self, id: u64, count: usize) -> IpcResponse {
        // 使用 file_logger 获取日志
        if let Ok(file_logger) = self.file_logger.try_lock() {
            let entries: Vec<LogEntryJson> = file_logger
                .get_recent_logs(count)
                .into_iter()
                .map(|e| LogEntryJson {
                    timestamp: e.timestamp.to_rfc3339(),
                    level: "INFO".to_string(),
                    source: "FTP".to_string(),
                    message: format!("{} {} {} - {}", e.username, e.operation, e.file_path, e.message),
                    client_ip: Some(e.client_ip),
                    username: Some(e.username),
                    action: Some(e.operation),
                })
                .collect();
            
            IpcResponse {
                id,
                result: crate::communication::protocol::IpcResult::Logs { entries },
            }
        } else {
            IpcResponse::error(id, "Failed to access logger")
        }
    }

    async fn handle_write_audit_log(&self, id: u64, user: &str, action: &str, target: &str, details: &str) -> IpcResponse {
        let entry = AuditEntry {
            timestamp: chrono::Utc::now().to_rfc3339(),
            user: user.to_string(),
            action: action.to_string(),
            target: target.to_string(),
            details: details.to_string(),
        };
        
        if let Some(parent) = Path::new(AUDIT_LOG_PATH).parent()
            && let Err(e) = fs::create_dir_all(parent) {
                return IpcResponse::error(id, &format!("Failed to create audit log directory: {}", e));
            }
        
        let log_line = match serde_json::to_string(&entry) {
            Ok(json) => json + "\n",
            Err(e) => return IpcResponse::error(id, &format!("Failed to serialize audit log: {}", e)),
        };
        
        match fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(AUDIT_LOG_PATH)
        {
            Ok(mut file) => {
                use std::io::Write;
                if let Err(e) = file.write_all(log_line.as_bytes()) {
                    return IpcResponse::error(id, &format!("Failed to write audit log: {}", e));
                }
                if let Err(e) = file.sync_all() {
                    return IpcResponse::error(id, &format!("Failed to sync audit log: {}", e));
                }
                IpcResponse::success(id, "Audit log written")
            }
            Err(e) => IpcResponse::error(id, &format!("Failed to open audit log: {}", e)),
        }
    }

    async fn handle_config_exists(&self, id: u64) -> IpcResponse {
        IpcResponse::bool_value(id, Path::new(CONFIG_PATH).exists())
    }

    async fn handle_users_exists(&self, id: u64) -> IpcResponse {
        IpcResponse::bool_value(id, Path::new(USERS_PATH).exists())
    }

    async fn handle_get_initial_state(&self, id: u64) -> IpcResponse {
        let config_content = fs::read_to_string(CONFIG_PATH).unwrap_or_default();
        let users_content = if Path::new(USERS_PATH).exists() {
            fs::read_to_string(USERS_PATH).unwrap_or_default()
        } else {
            "{}".to_string()
        };
        let ftp_running = self.server_manager.is_ftp_running();
        let sftp_running = self.server_manager.is_sftp_running();
        
        IpcResponse::initial_state(id, config_content, users_content, ftp_running, sftp_running)
    }

    async fn handle_ensure_user_directories(&self, id: u64) -> IpcResponse {
        let users = self.user_manager.lock().unwrap();
        let users_list = users.list_users();
        
        for (_, user) in users_list {
            let path = Path::new(&user.home_dir);
            if !path.exists()
                && let Err(e) = fs::create_dir_all(path) {
                    warn!(path = %user.home_dir, error = %e, "Failed to create directory");
                }
        }
        
        IpcResponse::success(id, "User directories ensured")
    }

    async fn handle_get_log_files(&self, id: u64) -> IpcResponse {
        let log_dir = {
            let config = self.config.lock().unwrap();
            config.logging.log_dir.clone()
        };
        
        let mut files = Vec::new();
        files.push(LogFileEntry {
            name: "当前日志 (内存缓冲)".to_string(),
            path: "current".to_string(),
        });
        
        if let Ok(entries) = fs::read_dir(&log_dir) {
            let mut log_files: Vec<LogFileEntry> = entries
                .filter_map(|e| e.ok())
                .filter(|e| {
                    let name = e.file_name();
                    let name = name.to_string_lossy();
                    name.starts_with("wftpg-") && name.ends_with(".log")
                })
                .map(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    let path = e.path().to_string_lossy().to_string();
                    LogFileEntry { name, path }
                })
                .collect();
            
            log_files.sort_by(|a, b| b.name.cmp(&a.name));
            files.extend(log_files);
        }
        
        IpcResponse::log_files(id, files)
    }

    async fn handle_get_log_file_content(&self, id: u64, path: &str, count: usize) -> IpcResponse {
        if path == "current" {
            // 使用 file_logger 获取日志
            if let Ok(file_logger) = self.file_logger.try_lock() {
                let entries: Vec<LogEntryJson> = file_logger
                    .get_recent_logs(count)
                    .into_iter()
                    .map(|e| LogEntryJson {
                        timestamp: e.timestamp.to_rfc3339(),
                        level: "INFO".to_string(),
                        source: "FTP".to_string(),
                        message: format!("{} {} {} - {}", e.username, e.operation, e.file_path, e.message),
                        client_ip: Some(e.client_ip),
                        username: Some(e.username),
                        action: Some(e.operation),
                    })
                    .collect();
                IpcResponse::logs(id, entries)
            } else {
                IpcResponse::error(id, "Failed to access logger")
            }
        } else {
            match fs::read_to_string(path) {
                Ok(content) => {
                    let entries: Vec<LogEntryJson> = content
                        .lines()
                        .rev()
                        .take(count)
                        .filter_map(|line| {
                            let value: serde_json::Value = serde_json::from_str(line).ok()?;
                            Some(LogEntryJson {
                                timestamp: value.get("timestamp")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("-")
                                    .to_string(),
                                level: value.get("level")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("INFO")
                                    .to_string(),
                                source: value.get("target")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("系统")
                                    .to_string(),
                                message: value.get("message")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("-")
                                    .to_string(),
                                client_ip: value.get("client_ip").and_then(|v| v.as_str()).map(|s| s.to_string()),
                                username: value.get("username").and_then(|v| v.as_str()).map(|s| s.to_string()),
                                action: value.get("action").and_then(|v| v.as_str()).map(|s| s.to_string()),
                            })
                        })
                        .collect();
                    IpcResponse::logs(id, entries)
                }
                Err(e) => IpcResponse::error(id, &format!("Failed to read log file: {}", e)),
            }
        }
    }

    async fn handle_get_file_log_files(&self, id: u64) -> IpcResponse {
        let log_dir = {
            let config = self.config.lock().unwrap();
            config.logging.log_dir.clone()
        };
        
        let mut files = Vec::new();
        files.push(LogFileEntry {
            name: "当前日志 (内存缓冲)".to_string(),
            path: "current".to_string(),
        });
        
        if let Ok(entries) = fs::read_dir(&log_dir) {
            let mut log_files: Vec<LogFileEntry> = entries
                .filter_map(|e| e.ok())
                .filter(|e| {
                    let name = e.file_name();
                    let name = name.to_string_lossy();
                    name.starts_with("file-ops-") && name.ends_with(".log")
                })
                .map(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    let path = e.path().to_string_lossy().to_string();
                    LogFileEntry { name, path }
                })
                .collect();
            
            log_files.sort_by(|a, b| b.name.cmp(&a.name));
            files.extend(log_files);
        }
        
        IpcResponse::file_log_files(id, files)
    }

    async fn handle_get_file_log_file_content(&self, id: u64, path: &str, count: usize) -> IpcResponse {
        use super::protocol::FileLogEntryJson;
        
        if path == "current" {
            let file_logger = self.file_logger.lock().unwrap();
            let entries = file_logger.get_recent_logs(count);
            let json_entries: Vec<FileLogEntryJson> = entries
                .into_iter()
                .rev()
                .map(|e| FileLogEntryJson {
                    timestamp: e.timestamp.to_rfc3339(),
                    username: e.username,
                    client_ip: e.client_ip,
                    operation: e.operation,
                    file_path: e.file_path,
                    file_size: e.file_size,
                    protocol: e.protocol,
                    success: e.success,
                    message: e.message,
                })
                .collect();
            IpcResponse {
                id,
                result: IpcResult::FileLogEntries { entries: json_entries },
            }
        } else {
            match fs::read_to_string(path) {
                Ok(content) => {
                    let entries: Vec<crate::core::file_logger::FileLogEntry> = content
                        .lines()
                        .rev()
                        .take(count)
                        .filter_map(|line| serde_json::from_str(line).ok())
                        .collect();
                    let json_entries: Vec<FileLogEntryJson> = entries
                        .into_iter()
                        .map(|e| FileLogEntryJson {
                            timestamp: e.timestamp.to_rfc3339(),
                            username: e.username,
                            client_ip: e.client_ip,
                            operation: e.operation,
                            file_path: e.file_path,
                            file_size: e.file_size,
                            protocol: e.protocol,
                            success: e.success,
                            message: e.message,
                        })
                        .collect();
                    IpcResponse {
                        id,
                        result: IpcResult::FileLogEntries { entries: json_entries },
                    }
                }
                Err(e) => IpcResponse::error(id, &format!("Failed to read file log: {}", e)),
            }
        }
    }

    async fn handle_save_log_config(
        &self, 
        id: u64, 
        log_dir: &str, 
        log_level: &str, 
        max_log_size: u64, 
        max_log_files: usize, 
        _log_to_file: bool, 
        enable_gui_logging: bool
    ) -> IpcResponse {
        let mut config = self.config.lock().unwrap().clone();
        config.logging.log_dir = log_dir.to_string();
        config.logging.log_level = log_level.to_string();
        config.logging.max_log_size = max_log_size;
        config.logging.max_log_files = max_log_files;
        config.logging.enable_gui_logging = enable_gui_logging;
        
        match config.save(&Config::get_config_path()) {
            Ok(()) => {
                let mut cfg = self.config.lock().unwrap();
                *cfg = config;
                IpcResponse::success(id, "Log configuration saved")
            }
            Err(e) => IpcResponse::error(id, &format!("Failed to save log config: {}", e)),
        }
    }

    async fn handle_setup_directory_permissions(&self, id: u64, path: &str) -> IpcResponse {
        let path = Path::new(path);
        
        if !path.exists() {
            return IpcResponse::error(id, "Directory does not exist");
        }
        
        let wftpg_group_exists = std::process::Command::new("getent")
            .args(["group", "wftpg"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        
        if !wftpg_group_exists {
            return IpcResponse::error(id, "wftpg group not found");
        }
        
        let chgrp_result = std::process::Command::new("chgrp")
            .args(["wftpg", &path.to_string_lossy()])
            .status();
        
        match chgrp_result {
            Ok(status) if status.success() => {
                info!("Changed group ownership to wftpg for {}", path.display());
            }
            _ => {
                warn!("Failed to chgrp directory to wftpg group");
            }
        }
        
        if let Err(e) = fs::set_permissions(path, fs::Permissions::from_mode(0o2770)) {
            warn!("Failed to set directory permissions: {}", e);
        }
        
        let setfacl_result = std::process::Command::new("setfacl")
            .args(["-d", "-m", "u::rw-,g::rw-,o::---", &path.to_string_lossy()])
            .status();
        
        match setfacl_result {
            Ok(status) if status.success() => {
                info!("Set default ACL for directory {}", path.display());
            }
            _ => {
                info!("setfacl not available, using umask for file permissions");
            }
        }
        
        IpcResponse::success(id, "Directory permissions set up successfully")
    }

    async fn handle_create_user_directory(&self, id: u64, path: &str) -> IpcResponse {
        let path = Path::new(path);
        
        if path.exists() {
            return IpcResponse::success(id, "Directory already exists");
        }
        
        match fs::create_dir_all(path) {
            Ok(()) => IpcResponse::success(id, "Directory created successfully"),
            Err(e) => IpcResponse::error(id, &format!("Failed to create directory: {}", e)),
        }
    }

    pub fn get_log_sender(&self) -> broadcast::Sender<LogEntryJson> {
        self.log_sender.clone()
    }
}

#[derive(Debug, Clone, serde::Serialize)]
struct AuditEntry {
    timestamp: String,
    user: String,
    action: String,
    target: String,
    details: String,
}
