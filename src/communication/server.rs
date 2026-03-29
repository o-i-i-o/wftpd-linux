use anyhow::Result;
use std::ffi::CString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::broadcast;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use super::protocol::*;
use crate::core::server_manager::ServerManager;
use crate::core::config::Config;
use crate::core::users::UserManager;
use crate::core::logger::Logger;
use crate::core::file_logger::FileLogger;
use crate::service::ServiceManager;

pub struct IpcServer {
    config: Arc<std::sync::Mutex<Config>>,
    user_manager: Arc<std::sync::Mutex<UserManager>>,
    server_manager: ServerManager,
    service_manager: ServiceManager,
    logger: Arc<std::sync::Mutex<Logger>>,
    file_logger: Arc<std::sync::Mutex<FileLogger>>,
    log_sender: broadcast::Sender<LogEntryJson>,
}

impl IpcServer {
    pub fn new(
        config: Arc<std::sync::Mutex<Config>>,
        user_manager: Arc<std::sync::Mutex<UserManager>>,
        server_manager: ServerManager,
        service_manager: ServiceManager,
        logger: Arc<std::sync::Mutex<Logger>>,
        file_logger: Arc<std::sync::Mutex<FileLogger>>,
    ) -> Self {
        let (log_sender, _) = broadcast::channel(256);
        
        Self {
            config,
            user_manager,
            server_manager,
            service_manager,
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
        fs::set_permissions(socket_path, fs::Permissions::from_mode(0o660))?;
        
        log::info!("IPC server listening on {}", SOCKET_PATH);
        
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let server = self.clone_handler();
                    tokio::spawn(async move {
                        if let Err(e) = server.handle_connection(stream).await {
                            log::error!("IPC connection error: {}", e);
                        }
                    });
                }
                Err(e) => {
                    log::error!("Failed to accept IPC connection: {}", e);
                }
            }
        }
    }

    fn clone_handler(&self) -> Self {
        Self {
            config: Arc::clone(&self.config),
            user_manager: Arc::clone(&self.user_manager),
            server_manager: self.server_manager.clone(),
            service_manager: ServiceManager::new(),
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
                    log::error!("Failed to read from IPC client: {}", e);
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
            IpcCommand::GetConfig => self.handle_get_config(request.id).await,
            IpcCommand::SaveConfig { content } => self.handle_save_config(request.id, content).await,
            IpcCommand::GetUsers => self.handle_get_users(request.id).await,
            IpcCommand::SaveUsers { content } => self.handle_save_users(request.id, content).await,
            IpcCommand::StartFtp => self.handle_start_ftp(request.id).await,
            IpcCommand::StopFtp => self.handle_stop_ftp(request.id).await,
            IpcCommand::StartSftp => self.handle_start_sftp(request.id).await,
            IpcCommand::StopSftp => self.handle_stop_sftp(request.id).await,
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
            IpcCommand::InstallService { binary_path } => self.handle_install_service(request.id, &binary_path).await,
            IpcCommand::UninstallService => self.handle_uninstall_service(request.id).await,
            IpcCommand::StartSystemService => self.handle_start_system_service(request.id).await,
            IpcCommand::StopSystemService => self.handle_stop_system_service(request.id).await,
            IpcCommand::RestartSystemService => self.handle_restart_system_service(request.id).await,
            IpcCommand::EnableService => self.handle_enable_service(request.id).await,
            IpcCommand::DisableService => self.handle_disable_service(request.id).await,
            IpcCommand::GetSystemServiceStatus => self.handle_get_system_service_status(request.id).await,
            IpcCommand::GetInitialState => self.handle_get_initial_state(request.id).await,
            IpcCommand::EnsureUserDirectories => self.handle_ensure_user_directories(request.id).await,
            IpcCommand::GetLogFiles => self.handle_get_log_files(request.id).await,
            IpcCommand::GetLogFileContent { path, count } => self.handle_get_log_file_content(request.id, &path, count).await,
            IpcCommand::GetFileLogFiles => self.handle_get_file_log_files(request.id).await,
            IpcCommand::GetFileLogFileContent { path, count } => self.handle_get_file_log_file_content(request.id, &path, count).await,
            IpcCommand::SaveLogConfig { log_dir, log_level, max_log_size, max_log_files, log_to_file, log_to_gui } => {
                self.handle_save_log_config(request.id, &log_dir, &log_level, max_log_size, max_log_files, log_to_file, log_to_gui).await
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
        if let Some(parent) = Path::new(USERS_PATH).parent()
            && let Err(e) = fs::create_dir_all(parent) {
                return IpcResponse::error(id, &format!("Failed to create users directory: {}", e));
            }
        
        match fs::write(USERS_PATH, &content) {
            Ok(()) => {
                let users_path = Config::get_users_path();
                match UserManager::load(&users_path) {
                    Ok(new_users) => {
                        {
                            let mut users = self.user_manager.lock().unwrap();
                            *users = new_users;
                        }
                        match fs::read_to_string(USERS_PATH) {
                            Ok(saved_content) => IpcResponse {
                                id,
                                result: IpcResult::UsersSaved {
                                    message: "Users saved".to_string(),
                                    content: saved_content,
                                },
                            },
                            Err(e) => IpcResponse::error(id, &format!("Failed to read saved users: {}", e)),
                        }
                    }
                    Err(e) => IpcResponse::error(id, &format!("Failed to load saved users: {}", e)),
                }
            }
            Err(e) => IpcResponse::error(id, &format!("Failed to save users: {}", e)),
        }
    }

    async fn handle_start_ftp(&self, id: u64) -> IpcResponse {
        match self.server_manager.start_ftp(
            Arc::clone(&self.config),
            Arc::clone(&self.user_manager),
            Arc::clone(&self.logger),
            Arc::clone(&self.file_logger),
        ).await {
            Ok(()) => IpcResponse::success(id, "FTP server started"),
            Err(e) => IpcResponse::error(id, &format!("Failed to start FTP: {}", e)),
        }
    }

    async fn handle_stop_ftp(&self, id: u64) -> IpcResponse {
        self.server_manager.stop_ftp(&self.logger).await;
        IpcResponse::success(id, "FTP server stopped")
    }

    async fn handle_start_sftp(&self, id: u64) -> IpcResponse {
        match self.server_manager.start_sftp(
            Arc::clone(&self.config),
            Arc::clone(&self.user_manager),
            Arc::clone(&self.logger),
            Arc::clone(&self.file_logger),
        ).await {
            Ok(()) => IpcResponse::success(id, "SFTP server started"),
            Err(e) => IpcResponse::error(id, &format!("Failed to start SFTP: {}", e)),
        }
    }

    async fn handle_stop_sftp(&self, id: u64) -> IpcResponse {
        self.server_manager.stop_sftp(&self.logger).await;
        IpcResponse::success(id, "SFTP server stopped")
    }

    async fn handle_restart_service(&self, id: u64) -> IpcResponse {
        self.server_manager.stop_ftp(&self.logger).await;
        self.server_manager.stop_sftp(&self.logger).await;
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        
        let (ftp_enabled, sftp_enabled) = {
            let cfg = self.config.lock().unwrap();
            (cfg.ftp.enabled, cfg.sftp.enabled)
        };
        
        if ftp_enabled
            && let Err(e) = self.server_manager.start_ftp(
                Arc::clone(&self.config),
                Arc::clone(&self.user_manager),
                Arc::clone(&self.logger),
                Arc::clone(&self.file_logger),
            ).await {
                return IpcResponse::error(id, &format!("Failed to start FTP: {}", e));
            }
        
        if sftp_enabled
            && let Err(e) = self.server_manager.start_sftp(
                Arc::clone(&self.config),
                Arc::clone(&self.user_manager),
                Arc::clone(&self.logger),
                Arc::clone(&self.file_logger),
            ).await {
                return IpcResponse::error(id, &format!("Failed to start SFTP: {}", e));
            }
        
        IpcResponse::success(id, "Service restarted")
    }

    async fn handle_get_status(&self, id: u64) -> IpcResponse {
        let ftp_running = self.server_manager.is_ftp_running();
        let sftp_running = self.server_manager.is_sftp_running();
        IpcResponse::status(id, ftp_running, sftp_running)
    }

    async fn handle_get_logs(&self, id: u64, count: usize) -> IpcResponse {
        let logger = self.logger.lock().unwrap();
        let entries: Vec<LogEntryJson> = logger
            .get_recent_logs(count)
            .into_iter()
            .map(|e| LogEntryJson {
                timestamp: e.timestamp.to_rfc3339(),
                level: e.level.to_string(),
                source: e.source,
                message: e.message,
                client_ip: e.client_ip,
                username: e.username,
                action: e.action,
            })
            .collect();
        IpcResponse::logs(id, entries)
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

    async fn handle_install_service(&self, id: u64, binary_path: &str) -> IpcResponse {
        match self.service_manager.install_service(binary_path) {
            Ok(_) => {
                let _ = self.service_manager.reload_daemon();
                IpcResponse::success(id, "Service installed successfully")
            }
            Err(e) => IpcResponse::error(id, &format!("Failed to install service: {}", e)),
        }
    }

    async fn handle_uninstall_service(&self, id: u64) -> IpcResponse {
        match self.service_manager.uninstall_service() {
            Ok(_) => IpcResponse::success(id, "Service uninstalled successfully"),
            Err(e) => IpcResponse::error(id, &format!("Failed to uninstall service: {}", e)),
        }
    }

    async fn handle_start_system_service(&self, id: u64) -> IpcResponse {
        match self.service_manager.start_service() {
            Ok(_) => IpcResponse::success(id, "Service started successfully"),
            Err(e) => IpcResponse::error(id, &format!("Failed to start service: {}", e)),
        }
    }

    async fn handle_stop_system_service(&self, id: u64) -> IpcResponse {
        match self.service_manager.stop_service() {
            Ok(_) => IpcResponse::success(id, "Service stopped successfully"),
            Err(e) => IpcResponse::error(id, &format!("Failed to stop service: {}", e)),
        }
    }

    async fn handle_restart_system_service(&self, id: u64) -> IpcResponse {
        match self.service_manager.restart_service() {
            Ok(_) => IpcResponse::success(id, "Service restarted successfully"),
            Err(e) => IpcResponse::error(id, &format!("Failed to restart service: {}", e)),
        }
    }

    async fn handle_enable_service(&self, id: u64) -> IpcResponse {
        match self.service_manager.enable_service() {
            Ok(_) => IpcResponse::success(id, "Service enabled successfully"),
            Err(e) => IpcResponse::error(id, &format!("Failed to enable service: {}", e)),
        }
    }

    async fn handle_disable_service(&self, id: u64) -> IpcResponse {
        match self.service_manager.disable_service() {
            Ok(_) => IpcResponse::success(id, "Service disabled successfully"),
            Err(e) => IpcResponse::error(id, &format!("Failed to disable service: {}", e)),
        }
    }

    async fn handle_get_system_service_status(&self, id: u64) -> IpcResponse {
        let installed = self.service_manager.service_exists();
        let running = self.service_manager.is_service_running();
        let enabled = self.service_manager.is_service_enabled();
        IpcResponse::service_status(id, installed, running, enabled)
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
                    log::warn!("Failed to create directory {}: {}", user.home_dir, e);
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
            let logger = self.logger.lock().unwrap();
            let entries: Vec<LogEntryJson> = logger
                .get_recent_logs(count)
                .into_iter()
                .map(|e| LogEntryJson {
                    timestamp: e.timestamp.to_rfc3339(),
                    level: e.level.to_string(),
                    source: e.source,
                    message: e.message,
                    client_ip: e.client_ip,
                    username: e.username,
                    action: e.action,
                })
                .collect();
            IpcResponse::logs(id, entries)
        } else {
            match fs::read_to_string(path) {
                Ok(content) => {
                    let entries: Vec<LogEntryJson> = content
                        .lines()
                        .rev()
                        .take(count)
                        .filter_map(|line| serde_json::from_str::<crate::core::logger::LogEntry>(line).ok())
                        .map(|e| LogEntryJson {
                            timestamp: e.timestamp.to_rfc3339(),
                            level: e.level.to_string(),
                            source: e.source,
                            message: e.message,
                            client_ip: e.client_ip,
                            username: e.username,
                            action: e.action,
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
        log_to_file: bool, 
        log_to_gui: bool
    ) -> IpcResponse {
        let mut config = self.config.lock().unwrap().clone();
        config.logging.log_dir = log_dir.to_string();
        config.logging.log_level = log_level.to_string();
        config.logging.max_log_size = max_log_size;
        config.logging.max_log_files = max_log_files;
        config.logging.log_to_file = log_to_file;
        config.logging.log_to_gui = log_to_gui;
        
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
                log::info!("Changed group ownership to wftpg for {}", path.display());
            }
            _ => {
                log::warn!("Failed to chgrp directory to wftpg group");
            }
        }
        
        if let Err(e) = fs::set_permissions(path, fs::Permissions::from_mode(0o2770)) {
            log::warn!("Failed to set directory permissions: {}", e);
        }
        
        let setfacl_result = std::process::Command::new("setfacl")
            .args(["-d", "-m", "u::rw-,g::rw-,o::---", &path.to_string_lossy()])
            .status();
        
        match setfacl_result {
            Ok(status) if status.success() => {
                log::info!("Set default ACL for directory {}", path.display());
            }
            _ => {
                log::info!("setfacl not available, using umask for file permissions");
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
