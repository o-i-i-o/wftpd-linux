//! libunftp 认证桥。
//!
//! - 密码认证走 `UserManager::authenticate`（argon2），认证前从磁盘重载用户库
//! - 匿名访问：`allow_anonymous = true` 时接受 anonymous/ftp 用户，
//!   由 `UserDetailProvider` 将其映射到 `anonymous_home`
//! - IP 过滤：libunftp 在 `Credentials` 中提供来源 IP，
//!   `allowed_ips` / `denied_ips` 规则在此执行（命中拒绝返回 IpDisallowed）

use async_trait::async_trait;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};
use tracing::{info, warn};
use unftp_core::auth::{
    AuthenticationError, Authenticator, Credentials, Principal, UserDetailError, UserDetailProvider,
};
use wftpd_common::{Config, UserManager};

/// 认证成功后用于组装 [`super::storage::WftpdUser`] 的用户详情提供者
#[derive(Debug)]
pub struct WftpdUserDetailProvider {
    user_manager: Arc<StdMutex<UserManager>>,
    allow_anonymous: bool,
    anonymous_home: Option<PathBuf>,
}

impl WftpdUserDetailProvider {
    pub fn new(config: &Config, user_manager: Arc<StdMutex<UserManager>>) -> Self {
        WftpdUserDetailProvider {
            user_manager,
            allow_anonymous: config.ftp.allow_anonymous,
            anonymous_home: config.ftp.anonymous_home.as_ref().map(PathBuf::from),
        }
    }
}

#[async_trait]
impl UserDetailProvider for WftpdUserDetailProvider {
    type User = super::storage::WftpdUser;

    async fn provide_user_detail(
        &self,
        principal: &Principal,
    ) -> Result<Self::User, UserDetailError> {
        // 匿名用户：主目录取 anonymous_home，权限全开（与匿名 FTP 语义一致）
        if principal.username == "anonymous" || principal.username == "ftp" {
            let home = self
                .anonymous_home
                .clone()
                .ok_or_else(|| UserDetailError::Generic("anonymous home not configured".into()))?;
            return Ok(super::storage::WftpdUser::anonymous(home));
        }

        let detail = {
            let users = self.user_manager.lock().unwrap();
            users
                .get_user(&principal.username)
                .map(|u| (PathBuf::from(&u.home_dir), u.permissions.clone(), u.enabled))
        };

        match detail {
            Some((home, permissions, enabled)) => {
                if !enabled {
                    return Err(UserDetailError::Generic(format!(
                        "account '{}' is disabled",
                        principal.username
                    )));
                }
                Ok(super::storage::WftpdUser::new(
                    principal.username.clone(),
                    home,
                    permissions,
                ))
            }
            None => Err(UserDetailError::UserNotFound {
                username: principal.username.clone(),
            }),
        }
    }
}

pub struct WftpdAuthenticator {
    user_manager: Arc<StdMutex<UserManager>>,
    allow_anonymous: bool,
    allowed_ips: Vec<String>,
    denied_ips: Vec<String>,
}

impl std::fmt::Debug for WftpdAuthenticator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WftpdAuthenticator").finish()
    }
}

impl WftpdAuthenticator {
    pub fn new(config: &Config, user_manager: Arc<StdMutex<UserManager>>) -> Self {
        WftpdAuthenticator {
            user_manager,
            allow_anonymous: config.ftp.allow_anonymous,
            allowed_ips: config.security.allowed_ips.clone(),
            denied_ips: config.security.denied_ips.clone(),
        }
    }

    fn ip_allowed(&self, ip: &IpAddr) -> bool {
        let ip_str = ip.to_string();
        wftpd_common::Config::is_ip_allowed_for(&self.allowed_ips, &self.denied_ips, &ip_str)
    }
}

#[async_trait]
impl Authenticator for WftpdAuthenticator {
    async fn authenticate(
        &self,
        username: &str,
        creds: &Credentials,
    ) -> Result<Principal, AuthenticationError> {
        if !self.ip_allowed(&creds.source_ip) {
            warn!(client_ip = %creds.source_ip, user = %username, "连接被 IP 过滤规则拒绝");
            return Err(AuthenticationError::IpDisallowed);
        }

        // 匿名访问
        if username == "anonymous" || username == "ftp" {
            if self.allow_anonymous {
                info!(client_ip = %creds.source_ip, "匿名用户登录");
                return Ok(Principal {
                    username: "anonymous".to_string(),
                });
            }
            return Err(AuthenticationError::BadUser);
        }

        let password = creds
            .password
            .as_deref()
            .ok_or(AuthenticationError::BadPassword)?;

        {
            let mut users = self.user_manager.lock().unwrap();
            // 与 SFTP 一致：认证前重载用户库，保证 GUI 改动即时生效
            if let Err(e) = users.reload(&wftpd_common::Config::get_users_path()) {
                warn!(error = %e, "重载用户配置失败，使用内存副本");
            }
            match users.authenticate(username, password) {
                Ok(true) => {
                    info!(user = %username, client_ip = %creds.source_ip, "FTP 用户登录成功");
                    Ok(Principal {
                        username: username.to_string(),
                    })
                }
                _ => {
                    info!(user = %username, client_ip = %creds.source_ip, "FTP 登录失败");
                    Err(AuthenticationError::BadPassword)
                }
            }
        }
    }
}
