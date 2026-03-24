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

pub struct IpcServer {
    config: Arc<std::sync::Mutex<Config>>,
    user_manager: Arc<std::sync::Mutex<UserManager>>,
    server_manager: ServerManager,
    logger: Arc<std::sync::Mutex<Logger>>,
    file_logger: Arc<std::sync::Mutex<FileLogger>>,
    log_sender: broadcast::Sender<LogEntryJson>,
}

impl IpcServer {
    pub fn new(
        config: Arc<std::sync::Mutex<Config>>,
        user_manager: Arc<std::sync::Mutex<UserManager>>,
        server_manager: ServerManager,
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
        
        match fs::write(CONFIG_PATH, content) {
            Ok(()) => {
                let config_path = Config::get_config_path();
                if let Ok(new_config) = Config::load(&config_path) {
                    let mut config = self.config.lock().unwrap();
                    *config = new_config;
                }
                IpcResponse::success(id, "Configuration saved")
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
        
        match fs::write(USERS_PATH, content) {
            Ok(()) => {
                let users_path = Config::get_users_path();
                if let Ok(new_users) = UserManager::load(&users_path) {
                    let mut users = self.user_manager.lock().unwrap();
                    *users = new_users;
                }
                IpcResponse::success(id, "Users saved")
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
