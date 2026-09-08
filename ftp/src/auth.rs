//! libunftp 认证桥。
//!
//! - 密码认证走 `UserManager::authenticate`（argon2），认证前从磁盘重载用户库
//! - 匿名访问：`allow_anonymous = true` 时接受 anonymous/ftp 用户，
//!   由 `UserDetailProvider` 将其映射到 `anonymous_home`
//! - IP 过滤：libunftp 在 `Credentials` 中提供来源 IP，
//!   `allowed_ips` / `denied_ips` 规则在此执行（命中拒绝返回 `IpDisallowed`）

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
    anonymous_home: Option<PathBuf>,
}

impl WftpdUserDetailProvider {
    pub fn new(config: &Config, user_manager: Arc<StdMutex<UserManager>>) -> Self {
        WftpdUserDetailProvider {
            user_manager,
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
                .map(|u| (PathBuf::from(&u.home_dir), u.permissions, u.enabled))
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
            if let Ok(true) = users.authenticate(username, password) {
                info!(user = %username, client_ip = %creds.source_ip, "FTP 用户登录成功");
                Ok(Principal {
                    username: username.to_string(),
                })
            } else {
                info!(user = %username, client_ip = %creds.source_ip, "FTP 登录失败");
                Err(AuthenticationError::BadPassword)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unftp_core::auth::{ChannelEncryptionState, UserDetailError};

    fn block_on<F: std::future::Future>(fut: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(fut)
    }

    fn config_with(allow_anonymous: bool, anonymous_home: Option<String>) -> Config {
        let mut config = Config::default();
        config.ftp.allow_anonymous = allow_anonymous;
        config.ftp.anonymous_home = anonymous_home;
        config.security.allowed_ips = vec!["0.0.0.0/0".to_string()];
        config.security.denied_ips = vec![];
        config
    }

    fn creds_from_ip(ip: &str) -> Credentials {
        Credentials {
            password: Some("pw".to_string()),
            certificate_chain: None,
            source_ip: ip.parse().unwrap(),
            command_channel_security: ChannelEncryptionState::Plaintext,
        }
    }

    // ---- WftpdUserDetailProvider ----

    #[test]
    fn provider_maps_anonymous_to_anonymous_home() {
        let dir = tempfile::tempdir().unwrap();
        let config = config_with(true, Some(dir.path().to_string_lossy().into_owned()));
        let manager = Arc::new(StdMutex::new(UserManager::new()));
        let provider = WftpdUserDetailProvider::new(&config, manager);

        for name in ["anonymous", "ftp"] {
            let user = block_on(provider.provide_user_detail(&Principal {
                username: name.to_string(),
            }))
            .expect("匿名用户应能获取详情");
            assert!(user.anonymous);
            assert_eq!(user.username, "anonymous");
            assert_eq!(user.home, dir.path());
            assert_eq!(user.permissions, wftpd_common::Permissions::full());
        }
    }

    #[test]
    fn provider_rejects_anonymous_without_home() {
        let config = config_with(true, None);
        let provider =
            WftpdUserDetailProvider::new(&config, Arc::new(StdMutex::new(UserManager::new())));
        let result = block_on(provider.provide_user_detail(&Principal {
            username: "anonymous".to_string(),
        }));
        assert!(result.is_err(), "未配置 anonymous_home 时匿名详情应失败");
    }

    #[test]
    fn provider_returns_real_user_details() {
        let dir = tempfile::tempdir().unwrap();
        let mut manager = UserManager::new();
        manager
            .add_user(
                "alice".into(),
                "pw",
                dir.path().to_string_lossy().into_owned(),
                wftpd_common::Permissions::full(),
                false,
            )
            .unwrap();
        let manager = Arc::new(StdMutex::new(manager));
        let provider =
            WftpdUserDetailProvider::new(&config_with(false, None), Arc::clone(&manager));

        let user = block_on(provider.provide_user_detail(&Principal {
            username: "alice".to_string(),
        }))
        .expect("已存在用户应返回详情");
        assert!(!user.anonymous);
        assert_eq!(user.home, dir.path());
        assert!(user.permissions.can_write);
    }

    #[test]
    fn provider_rejects_disabled_user() {
        let dir = tempfile::tempdir().unwrap();
        let mut manager = UserManager::new();
        manager
            .add_user(
                "bob".into(),
                "pw",
                dir.path().to_string_lossy().into_owned(),
                wftpd_common::Permissions::full(),
                false,
            )
            .unwrap();
        manager.set_user_enabled("bob", false).unwrap();
        let provider = WftpdUserDetailProvider::new(
            &config_with(false, None),
            Arc::new(StdMutex::new(manager)),
        );

        assert!(
            block_on(provider.provide_user_detail(&Principal {
                username: "bob".to_string(),
            }))
            .is_err()
        );
    }

    #[test]
    fn provider_rejects_unknown_user() {
        let provider = WftpdUserDetailProvider::new(
            &config_with(false, None),
            Arc::new(StdMutex::new(UserManager::new())),
        );
        let result = block_on(provider.provide_user_detail(&Principal {
            username: "ghost".to_string(),
        }));
        assert!(matches!(result, Err(UserDetailError::UserNotFound { .. })));
    }

    // ---- WftpdAuthenticator ----

    #[test]
    fn authenticator_rejects_denied_ip() {
        let mut config = config_with(false, None);
        config.security.denied_ips = vec!["10.66.0.0/16".to_string()];
        let authenticator =
            WftpdAuthenticator::new(&config, Arc::new(StdMutex::new(UserManager::new())));

        let result = block_on(authenticator.authenticate("alice", &creds_from_ip("10.66.1.1")));
        assert!(matches!(result, Err(AuthenticationError::IpDisallowed)));

        let ok = block_on(authenticator.authenticate("alice", &creds_from_ip("10.67.1.1")));
        assert!(!matches!(ok, Err(AuthenticationError::IpDisallowed)));
    }

    #[test]
    fn authenticator_anonymous_when_allowed() {
        let config = config_with(true, Some("/srv/ftp".to_string()));
        let authenticator =
            WftpdAuthenticator::new(&config, Arc::new(StdMutex::new(UserManager::new())));

        let principal =
            block_on(authenticator.authenticate("anonymous", &creds_from_ip("127.0.0.1")))
                .expect("允许匿名时应认证通过");
        assert_eq!(principal.username, "anonymous");

        let ftp_principal =
            block_on(authenticator.authenticate("ftp", &creds_from_ip("127.0.0.1")))
                .expect("ftp 用户名同样映射为匿名");
        assert_eq!(ftp_principal.username, "anonymous");
    }

    #[test]
    fn authenticator_anonymous_rejected_when_disabled() {
        let config = config_with(false, None);
        let authenticator =
            WftpdAuthenticator::new(&config, Arc::new(StdMutex::new(UserManager::new())));

        let result = block_on(authenticator.authenticate("anonymous", &creds_from_ip("127.0.0.1")));
        assert!(matches!(result, Err(AuthenticationError::BadUser)));
    }

    #[test]
    fn authenticator_requires_password() {
        let config = config_with(false, None);
        let authenticator =
            WftpdAuthenticator::new(&config, Arc::new(StdMutex::new(UserManager::new())));

        let mut creds = creds_from_ip("127.0.0.1");
        creds.password = None;
        let result = block_on(authenticator.authenticate("alice", &creds));
        assert!(matches!(result, Err(AuthenticationError::BadPassword)));
    }

    #[test]
    fn authenticator_wrong_password_fails() {
        let config = config_with(false, None);
        let authenticator =
            WftpdAuthenticator::new(&config, Arc::new(StdMutex::new(UserManager::new())));

        let mut creds = creds_from_ip("127.0.0.1");
        creds.password = Some("definitely-wrong".to_string());
        let result = block_on(authenticator.authenticate("alice", &creds));
        assert!(matches!(result, Err(AuthenticationError::BadPassword)));
    }
}
