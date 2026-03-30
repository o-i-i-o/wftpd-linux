use anyhow::Result;
use russh::*;
use russh::keys::*;
use russh::server::Msg;
use russh::ChannelId;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use tokio::sync::Mutex;

use tracing::{info, debug, warn, error};

use super::state::SftpState;
use crate::core::users::UserManager;
use crate::core::file_logger::FileLogger;
use crate::server::common::utils::is_safe_username;
use crate::server::common::quota::QuotaCache;

pub struct SftpHandler {
    user_manager: Arc<StdMutex<UserManager>>,
    file_logger: Arc<StdMutex<FileLogger>>,
    quota_cache: Arc<QuotaCache>,
    authenticated: bool,
    username: Option<String>,
    home_dir: Option<String>,
    sftp_channel: Option<ChannelId>,
    sftp_state: Option<Arc<Mutex<SftpState>>>,
    client_ip: String,
    users_path: std::path::PathBuf,
    keys_dir: PathBuf,
}

impl SftpHandler {
    pub fn new(
        user_manager: Arc<StdMutex<UserManager>>,
        file_logger: Arc<StdMutex<FileLogger>>,
        quota_cache: Arc<QuotaCache>,
        client_ip: String,
        users_path: std::path::PathBuf,
        keys_dir: PathBuf,
    ) -> Self {
        SftpHandler {
            user_manager,
            file_logger,
            quota_cache,
            authenticated: false,
            username: None,
            home_dir: None,
            sftp_channel: None,
            sftp_state: None,
            client_ip,
            users_path,
            keys_dir,
        }
    }

    fn log_client_action(
        &self,
        action: &str,
        message: &str,
        username: Option<&str>,
        log_type: &str,
    ) {
        // 使用 tracing 记录文件操作审计日志
        if let Ok(mut file_log) = self.file_logger.try_lock() {
            file_log.log(crate::core::file_logger::FileLogInfo {
                username: username.unwrap_or("unknown"),
                client_ip: &self.client_ip,
                operation: action,
                file_path: message,
                file_size: 0,
                protocol: "SFTP",
                success: true,
                message: log_type,
            });
        }
    }

    fn log_warning(&self, message: &str) {
        // 使用 tracing 记录警告日志
        warn!(target: "sftp", client_ip = %self.client_ip, "{}", message);
    }

    fn log_info(&self, message: &str) {
        // 使用 tracing 记录信息日志
        info!(target: "sftp", client_ip = %self.client_ip, "{}", message);
    }

    async fn validate_and_set_home_dir(
        &mut self,
        user: &str,
        home_dir: &str,
    ) -> Result<PathBuf, String> {
        if home_dir.trim().is_empty() {
            return Err(format!("home directory not configured for user '{}'", user));
        }

        let home = PathBuf::from(home_dir);
        
        match tokio::fs::metadata(&home).await {
            Ok(metadata) => {
                if !metadata.is_dir() {
                    return Err(format!("home path '{}' is not a directory", home_dir));
                }
            }
            Err(e) => {
                return Err(format!("home directory '{}' does not exist or cannot be accessed: {}", home_dir, e));
            }
        }

        match tokio::fs::canonicalize(&home).await {
            Ok(home_canon) => {
                self.home_dir = Some(home_canon.to_string_lossy().to_string());
                Ok(home_canon)
            }
            Err(e) => {
                Err(format!("cannot canonicalize home directory '{}': {}", home.display(), e))
            }
        }
    }
}

impl russh::server::Handler for SftpHandler {
    type Error = anyhow::Error;

    async fn auth_password(
        &mut self,
        user: &str,
        password: &str,
    ) -> Result<server::Auth, Self::Error> {
        if !is_safe_username(user) {
            self.log_client_action(
                "SFTP",
                "Password auth failed: invalid username format",
                None,
                "AUTH_FAIL",
            );
            return Ok(server::Auth::Reject { 
                proceed_with_methods: None,
                partial_success: false,
            });
        }
        
        debug!(user = %user, ip = %self.client_ip, "[SFTP AUTH] 用户尝试密码认证");
        
        let auth_result = {
            match self.user_manager.try_lock() {
                Ok(mut users) => {
                    // Always reload users from disk to ensure latest data
                    debug!(users_path = ?self.users_path, "[SFTP AUTH] 重新加载用户配置文件");
                    if let Err(e) = users.reload(&self.users_path) {
                        error!(error = %e, "[SFTP AUTH] 重新加载用户配置失败");
                    }
                    let user_count = users.get_users().len();
                    debug!(user_count = user_count, "[SFTP AUTH] 当前内存中用户数");
                    
                    // 检查用户是否存在
                    if let Some(u) = users.get_user(user) {
                        debug!(
                            user = %user,
                            enabled = u.enabled,
                            home_dir = %u.home_dir,
                            "[SFTP AUTH] 找到用户"
                        );
                    } else {
                        warn!(user = %user, "[SFTP AUTH] 未找到用户");
                    }
                    
                    match users.authenticate(user, password) {
                        Ok(true) => {
                            info!(user = %user, "[SFTP AUTH] 用户密码认证成功");
                            self.authenticated = true;
                            self.username = Some(user.to_string());
                            users.get_user(user).map(|u| u.home_dir.clone())
                        }
                        Ok(false) | Err(_) => {
                            info!(user = %user, "[SFTP AUTH] 用户密码认证失败");
                            None
                        },
                    }
                }
                Err(_) => {
                    self.log_warning("Failed to acquire user_manager lock during password auth");
                    error!("[SFTP AUTH] 获取 UserManager 锁失败");
                    return Ok(server::Auth::Reject { 
                        proceed_with_methods: None,
                        partial_success: false,
                    });
                }
            }
        };
        
        match auth_result {
            Some(home_dir) => {
                match self.validate_and_set_home_dir(user, &home_dir).await {
                    Ok(_) => {
                        self.log_client_action(
                            "SFTP",
                            &format!("User {} logged in", user),
                            Some(user),
                            "LOGIN",
                        );
                        Ok(server::Auth::Accept)
                    }
                    Err(err_msg) => {
                        self.log_client_action(
                            "SFTP",
                            &format!("Login failed: {}", err_msg),
                            Some(user),
                            "LOGIN_FAIL",
                        );
                        Ok(server::Auth::Reject { 
                            proceed_with_methods: None,
                            partial_success: false,
                        })
                    }
                }
            }
            None => {
                self.log_client_action(
                    "SFTP",
                    &format!("Failed login attempt for user {}", user),
                    Some(user),
                    "AUTH_FAIL",
                );
                Ok(server::Auth::Reject { 
                    proceed_with_methods: None,
                    partial_success: false,
                })
            }
        }
    }

    async fn auth_publickey(
        &mut self,
        user: &str,
        public_key: &PublicKey,
    ) -> Result<server::Auth, Self::Error> {
        if !is_safe_username(user) {
            self.log_client_action(
                "SFTP",
                "Public key auth failed: invalid username format",
                None,
                "AUTH_FAIL",
            );
            return Ok(server::Auth::Reject { 
                proceed_with_methods: None,
                partial_success: false,
            });
        }
        
        debug!(user = %user, ip = %self.client_ip, "[SFTP AUTH] 用户尝试公钥认证");
        
        // Always reload users from disk to ensure latest data
        {
            match self.user_manager.try_lock() {
                Ok(mut users) => {
                    debug!(users_path = ?self.users_path, "[SFTP AUTH] 重新加载用户配置文件");
                    if let Err(e) = users.reload(&self.users_path) {
                        error!(error = %e, "[SFTP AUTH] 重新加载用户配置失败");
                    }
                    let user_count = users.get_users().len();
                    debug!(user_count = user_count, "[SFTP AUTH] 当前内存中用户数");
                }
                Err(_) => {
                    self.log_warning("Failed to acquire user_manager lock during public key auth");
                    error!("[SFTP AUTH] 获取 UserManager 锁失败");
                }
            }
        }
        
        let (enabled, user_home_dir) = {
            match self.user_manager.try_lock() {
                Ok(users) => {
                    if let Some(u) = users.get_user(user) {
                        debug!(
                            user = %user,
                            enabled = u.enabled,
                            home_dir = %u.home_dir,
                            "[SFTP AUTH] 找到用户"
                        );
                        (u.enabled, u.home_dir.clone())
                    } else {
                        warn!(user = %user, "[SFTP AUTH] 未找到用户或用户已禁用");
                        (false, String::new())
                    }
                }
                Err(_) => {
                    self.log_warning("Failed to acquire user_manager lock during public key auth");
                    error!("[SFTP AUTH] 获取 UserManager 锁失败 (检查用户)");
                    return Ok(server::Auth::Reject { 
                        proceed_with_methods: None,
                        partial_success: false,
                    });
                }
            }
        };
        
        if !enabled {
            info!(user = %user, "[SFTP AUTH] 用户公钥认证失败：用户未找到或已禁用");
            self.log_client_action(
                "SFTP",
                &format!("Public key auth failed for user {}: user not found or disabled", user),
                Some(user),
                "AUTH_FAIL",
            );
            return Ok(server::Auth::Reject { 
                proceed_with_methods: None,
                partial_success: false,
            });
        }
        
        let user_pubkey_path = self.keys_dir.join(format!("{}.pub", user));
        
        match tokio::fs::read_to_string(&user_pubkey_path).await {
            Ok(stored_key) => {
                match keys::parse_public_key_base64(stored_key.trim()) {
                    Ok(stored_pubkey) => {
                        if public_key == &stored_pubkey {
                            self.authenticated = true;
                            self.username = Some(user.to_string());
                            
                            match self.validate_and_set_home_dir(user, &user_home_dir).await {
                                Ok(_) => {
                                    self.log_client_action(
                                        "SFTP",
                                        &format!("User {} logged in via public key", user),
                                        Some(user),
                                        "LOGIN",
                                    );
                                    return Ok(server::Auth::Accept);
                                }
                                Err(err_msg) => {
                                    self.log_client_action(
                                        "SFTP",
                                        &format!("Login failed: {}", err_msg),
                                        Some(user),
                                        "LOGIN_FAIL",
                                    );
                                    return Ok(server::Auth::Reject { 
                                        proceed_with_methods: None,
                                        partial_success: false,
                                    });
                                }
                            }
                        }
                    }
                    Err(e) => {
                        self.log_client_action(
                            "SFTP",
                            &format!("Failed to parse stored public key for user {}: {}", user, e),
                            Some(user),
                            "AUTH_ERROR",
                        );
                    }
                }
            }
            Err(e) => {
                self.log_client_action(
                    "SFTP",
                    &format!("Failed to read public key file for user {}: {}", user, e),
                    Some(user),
                    "AUTH_ERROR",
                );
            }
        }

        self.log_client_action(
            "SFTP",
            &format!("Public key auth failed for user {}: key mismatch or not found", user),
            Some(user),
            "AUTH_FAIL",
        );

        Ok(server::Auth::Reject { 
            proceed_with_methods: None,
            partial_success: false,
        })
    }

    async fn channel_open_session(
        &mut self,
        _channel: Channel<Msg>,
        _session: &mut server::Session,
    ) -> Result<bool, Self::Error> {
        Ok(self.authenticated)
    }

    async fn subsystem_request(
        &mut self,
        channel: ChannelId,
        name: &str,
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        if name != "sftp" {
            self.log_warning(&format!("Unknown subsystem request: {}", name));
            let _ = session.channel_failure(channel);
            return Ok(());
        }

        if !self.authenticated {
            self.log_warning("SFTP subsystem request denied: not authenticated");
            let _ = session.channel_failure(channel);
            return Ok(());
        }

        let home_dir = match &self.home_dir {
            Some(h) => h.clone(),
            None => {
                self.log_warning("SFTP subsystem request denied: home_dir not set");
                let _ = session.channel_failure(channel);
                return Ok(());
            }
        };

        let username = self.username.clone();
        
        self.log_info(&format!("SFTP subsystem initialized for user: {:?}, home_dir: {}", username, home_dir));
        
        let _ = session.channel_success(channel);
        self.sftp_channel = Some(channel);
        
        self.sftp_state = Some(Arc::new(Mutex::new(SftpState::new(
            home_dir,
            username,
            Arc::clone(&self.user_manager),
            Arc::clone(&self.file_logger),
            Arc::clone(&self.quota_cache),
            self.client_ip.clone(),
        ))));
        
        Ok(())
    }

    async fn pty_request(
        &mut self,
        _channel: ChannelId,
        _term: &str,
        _col_width: u32,
        _row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(Pty, u32)],
        _session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn shell_request(
        &mut self,
        _channel: ChannelId,
        _session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        if self.sftp_channel == Some(channel)
            && let Some(state) = &self.sftp_state {
                let response = {
                    let mut sftp_state = state.lock().await;
                    sftp_state.process_sftp_data(data).await
                };
                
                if let Ok(resp) = response
                    && !resp.is_empty() {
                        let _ = session.data(channel, resp);
                    }
            }
        Ok(())
    }

    async fn channel_eof(
        &mut self,
        channel: ChannelId,
        _session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        if self.sftp_channel == Some(channel)
            && let Some(state) = &self.sftp_state {
                let mut sftp_state = state.lock().await;
                let buffer_len = sftp_state.buffer.len();
                if buffer_len > 0 {
                    self.log_warning(&format!(
                        "Channel EOF received with {} bytes of incomplete data in buffer, clearing",
                        buffer_len
                    ));
                    sftp_state.buffer.clear();
                }
            }
        Ok(())
    }

    async fn channel_close(
        &mut self,
        channel: ChannelId,
        _session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        if self.sftp_channel == Some(channel) {
            if let Some(state) = &self.sftp_state {
                let mut sftp_state = state.lock().await;
                let buffer_len = sftp_state.buffer.len();
                if buffer_len > 0 {
                    self.log_warning(&format!(
                        "Channel closed with {} bytes of incomplete data in buffer, clearing",
                        buffer_len
                    ));
                    sftp_state.buffer.clear();
                }
            }
            self.sftp_channel = None;
            self.sftp_state = None;
        }
        Ok(())
    }
}
