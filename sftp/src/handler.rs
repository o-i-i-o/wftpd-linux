//! russh 服务端处理器：SSH 认证与 SFTP 子系统接入。
//!
//! 认证成功后，SFTP 子系统请求会把 SSH 通道转换为字节流
//! （`Channel::into_stream`），交给 `russh_sftp::server::run` 处理
//! （见 [`ops::SftpFileHandler`]），本模块不再解析任何 SFTP 包。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};

use russh::keys::PublicKey;
use russh::server::{Auth, Msg, Session};
use russh::{Channel, ChannelId};
use tracing::{debug, error, info, warn};
use wftpd_common::server::quota::QuotaCache;
use wftpd_common::server::utils::is_safe_username;
use wftpd_common::{FileLogger, UserManager};

use crate::ops::SftpFileHandler;

/// 每个 SSH 连接一个实例，负责认证与会话生命周期
pub struct SftpHandler {
    user_manager: Arc<StdMutex<UserManager>>,
    file_logger: Arc<StdMutex<FileLogger>>,
    quota_cache: Arc<QuotaCache>,
    client_ip: String,
    users_path: PathBuf,
    keys_dir: PathBuf,
    authenticated: bool,
    username: Option<String>,
    home_dir: Option<String>,
    /// `channel_open_session` 收到的通道，sftp 子系统请求时取出转换为字节流
    open_channels: HashMap<ChannelId, Channel<Msg>>,
}

impl SftpHandler {
    pub fn new(
        user_manager: Arc<StdMutex<UserManager>>,
        file_logger: Arc<StdMutex<FileLogger>>,
        quota_cache: Arc<QuotaCache>,
        client_ip: String,
        users_path: PathBuf,
        keys_dir: PathBuf,
    ) -> Self {
        SftpHandler {
            user_manager,
            file_logger,
            quota_cache,
            client_ip,
            users_path,
            keys_dir,
            authenticated: false,
            username: None,
            home_dir: None,
            open_channels: HashMap::new(),
        }
    }

    fn audit(&self, message: &str, username: Option<&str>, result: &str) {
        if let Ok(mut file_log) = self.file_logger.try_lock() {
            file_log.log(&wftpd_common::FileLogInfo {
                username: username.unwrap_or("unknown"),
                client_ip: &self.client_ip,
                operation: "SFTP",
                file_path: message,
                file_size: 0,
                protocol: "SFTP",
                success: result == "LOGIN",
                message: result,
            });
        }
    }

    fn resolve_user(&self, user: &str) -> Option<(String, bool)> {
        if !is_safe_username(user) {
            warn!(user = %user, client_ip = %self.client_ip, "[SFTP AUTH] 用户名格式无效");
            return None;
        }

        {
            let mut users = self.user_manager.lock().unwrap();
            if let Err(e) = users.reload(&self.users_path) {
                error!(error = %e, "[SFTP AUTH] 重新加载用户配置失败");
            }
        }

        let users = self.user_manager.lock().unwrap();
        users
            .get_user(user)
            .filter(|u| u.enabled)
            .map(|u| (u.home_dir.clone(), u.enabled))
    }

    async fn validate_and_set_home_dir(
        &mut self,
        user: &str,
        home_dir: &str,
    ) -> Result<(), String> {
        if home_dir.trim().is_empty() {
            return Err(format!("home directory not configured for user '{user}'"));
        }

        let home = PathBuf::from(home_dir);

        let metadata = tokio::fs::metadata(&home).await.map_err(|e| {
            format!(
                "home directory '{}' does not exist or cannot be accessed: {e}",
                home.display()
            )
        })?;
        if !metadata.is_dir() {
            return Err(format!("home path '{home_dir}' is not a directory"));
        }

        let home_canon = tokio::fs::canonicalize(&home).await.map_err(|e| {
            format!(
                "cannot canonicalize home directory '{}': {e}",
                home.display()
            )
        })?;
        self.home_dir = Some(home_canon.to_string_lossy().into_owned());
        self.username = Some(user.to_string());
        self.authenticated = true;
        Ok(())
    }

    fn reject(&self, user: &str, via: &str) -> Auth {
        self.audit(
            &format!("{via} auth failed for user {user}"),
            Some(user),
            "AUTH_FAIL",
        );
        Auth::Reject {
            proceed_with_methods: None,
            partial_success: false,
        }
    }
}

impl russh::server::Handler for SftpHandler {
    type Error = anyhow::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        info!(user = %user, client_ip = %self.client_ip, "[SFTP AUTH] 密码认证请求");

        let Some((home_dir, _)) = self.resolve_user(user) else {
            return Ok(self.reject(user, "password"));
        };

        let ok = {
            let mut users = self.user_manager.lock().unwrap();
            matches!(users.authenticate(user, password), Ok(true))
        };

        if !ok {
            return Ok(self.reject(user, "password"));
        }

        match self.validate_and_set_home_dir(user, &home_dir).await {
            Ok(()) => {
                self.audit(&format!("User {user} logged in"), Some(user), "LOGIN");
                info!(user = %user, "[SFTP AUTH] 密码认证成功");
                Ok(Auth::Accept)
            }
            Err(err_msg) => {
                error!(user = %user, "[SFTP AUTH] 主目录验证失败：{err_msg}");
                self.audit(&format!("Login failed: {err_msg}"), Some(user), "AUTH_FAIL");
                Ok(Auth::Reject {
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
    ) -> Result<Auth, Self::Error> {
        info!(user = %user, client_ip = %self.client_ip, "[SFTP AUTH] 公钥认证请求");

        let Some((home_dir, enabled)) = self.resolve_user(user) else {
            return Ok(self.reject(user, "public key"));
        };
        if !enabled {
            return Ok(self.reject(user, "public key"));
        }

        let user_pubkey_path = self.keys_dir.join(format!("{user}.pub"));
        let key_ok = match tokio::fs::read_to_string(&user_pubkey_path).await {
            Ok(stored_key) => match russh::keys::parse_public_key_base64(stored_key.trim()) {
                Ok(stored_pubkey) => public_key == &stored_pubkey,
                Err(e) => {
                    error!(user = %user, error = %e, "[SFTP AUTH] 解析用户公钥失败");
                    false
                }
            },
            Err(_) => false,
        };

        if !key_ok {
            return Ok(self.reject(user, "public key"));
        }

        match self.validate_and_set_home_dir(user, &home_dir).await {
            Ok(()) => {
                self.audit(
                    &format!("User {user} logged in via public key"),
                    Some(user),
                    "LOGIN",
                );
                Ok(Auth::Accept)
            }
            Err(err_msg) => {
                error!(user = %user, "[SFTP AUTH] 主目录验证失败：{err_msg}");
                self.audit(&format!("Login failed: {err_msg}"), Some(user), "AUTH_FAIL");
                Ok(Auth::Reject {
                    proceed_with_methods: None,
                    partial_success: false,
                })
            }
        }
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: russh::server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        debug!(channel_id = ?channel.id(), "[SFTP CHANNEL] session channel opened");
        self.open_channels.insert(channel.id(), channel);
        // russh 0.63 起需显式通过 ChannelOpenHandle 接受通道打开请求
        reply.accept().await;
        Ok(())
    }

    async fn subsystem_request(
        &mut self,
        channel_id: ChannelId,
        name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if name != "sftp" {
            warn!(client_ip = %self.client_ip, "[SFTP] 拒绝非 sftp 子系统: {name}");
            let _ = session.channel_failure(channel_id);
            return Ok(());
        }

        if !self.authenticated {
            warn!(client_ip = %self.client_ip, "[SFTP] 未认证，拒绝 sftp 子系统");
            let _ = session.channel_failure(channel_id);
            return Ok(());
        }

        let Some(channel) = self.open_channels.remove(&channel_id) else {
            warn!(client_ip = %self.client_ip, "[SFTP] sftp 子系统请求未找到通道");
            let _ = session.channel_failure(channel_id);
            return Ok(());
        };

        let username = self.username.clone().unwrap_or_default();
        let home_dir = self.home_dir.clone().unwrap_or_default();

        info!(user = %username, client_ip = %self.client_ip, "[SFTP] 初始化 sftp 子系统");

        let handler = SftpFileHandler::new(
            username,
            home_dir,
            Arc::clone(&self.user_manager),
            Arc::clone(&self.file_logger),
            Arc::clone(&self.quota_cache),
            self.client_ip.clone(),
        );

        session.channel_success(channel_id)?;
        // russh_sftp::server::run 内部 spawn 任务，持续处理该通道直至关闭
        russh_sftp::server::run(channel.into_stream(), handler).await;
        Ok(())
    }
}
