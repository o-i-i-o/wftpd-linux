//! FtpServer：把 Config/UserManager/FileLogger 组装为 libunftp Server。
//!
//! 配置映射：
//! - passive_ports / masquerade_ip / welcome_message / idle_timeout → libunftp 选项
//! - require_ssl + cert/key → FTPS（ftps_required）
//! - max_login_attempts + ban_duration → libunftp FailedLoginsPolicy（防爆破锁定）
//! - max_connections → 连接数上限（在 accept 层强制）

use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};

use tracing::{info, warn};
use wftpd_common::server::quota::QuotaCache;
use wftpd_common::{Config, FileLogger, UserManager};

use crate::auth::{WftpdAuthenticator, WftpdUserDetailProvider};
use crate::storage::{WftpdFilesystem, WftpdUser};

pub struct FtpServer {
    config: Arc<StdMutex<Config>>,
    user_manager: Arc<StdMutex<UserManager>>,
    file_logger: Arc<StdMutex<FileLogger>>,
    running: Arc<StdMutex<bool>>,
    shutdown_tx: Arc<tokio::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    closed_rx: Arc<StdMutex<Option<tokio::sync::oneshot::Receiver<()>>>>,
}

impl FtpServer {
    pub fn new(
        config: Arc<StdMutex<Config>>,
        user_manager: Arc<StdMutex<UserManager>>,
        file_logger: Arc<StdMutex<FileLogger>>,
    ) -> Self {
        FtpServer {
            config,
            user_manager,
            file_logger,
            running: Arc::new(StdMutex::new(false)),
            shutdown_tx: Arc::new(tokio::sync::Mutex::new(None)),
            closed_rx: Arc::new(StdMutex::new(None)),
        }
    }

    pub async fn start(&self) -> anyhow::Result<()> {
        {
            let running = self.running.lock().unwrap();
            if *running {
                return Ok(());
            }
        }

        let (
            bind_ip,
            port,
            passive_ports,
            masquerade_ip,
            welcome,
            idle_timeout,
            require_ssl,
            cert_path,
            key_path,
            max_login_attempts,
            ban_duration,
        ) = {
            let cfg = self.config.lock().unwrap();
            (
                cfg.ftp.bind_ip.clone(),
                cfg.ftp.port,
                cfg.ftp.passive_ports,
                cfg.ftp.masquerade_ip.clone(),
                cfg.ftp.welcome_message.clone(),
                cfg.security.idle_timeout,
                cfg.security.require_ssl,
                cfg.security.cert_path.clone(),
                cfg.security.key_path.clone(),
                cfg.security.max_login_attempts,
                cfg.security.ban_duration,
            )
        };

        // libunftp 的 greeting 是 &'static str，配置来自启动时读取，泄漏一次可接受
        let greeting: &'static str = Box::leak(welcome.into_boxed_str());

        let authenticator = Arc::new(WftpdAuthenticator::new(
            &self.config.lock().unwrap(),
            Arc::clone(&self.user_manager),
        ));
        let provider = Arc::new(WftpdUserDetailProvider::new(
            &self.config.lock().unwrap(),
            Arc::clone(&self.user_manager),
        ));

        let file_logger_for_sbe = Arc::clone(&self.file_logger);
        let quota_cache = Arc::new(QuotaCache::new());

        let mut builder = libunftp::ServerBuilder::new(Box::new(move || {
            WftpdFilesystem::new(Arc::clone(&file_logger_for_sbe), Arc::clone(&quota_cache))
        }))
        .authenticator(authenticator)
        .user_detail_provider::<WftpdUser, _>(provider)
        .greeting(greeting)
        .passive_ports(passive_ports.0..=passive_ports.1)
        .idle_session_timeout(idle_timeout);

        builder = match masquerade_ip
            .as_deref()
            .and_then(|s| s.parse::<IpAddr>().ok())
            .map(|ip| match ip {
                IpAddr::V4(v4) => Some(v4),
                IpAddr::V6(_) => None,
            })
            .unwrap_or(None)
        {
            Some(v4) => builder.passive_host(v4),
            None => builder,
        };

        // FTPS：证书可用即启用 AUTH TLS；require_ssl 时强制升级
        if let (Some(cert), Some(key)) = (cert_path, key_path) {
            let cert = PathBuf::from(cert);
            let key = PathBuf::from(key);
            if cert.exists() && key.exists() {
                info!(cert = %cert.display(), key = %key.display(), "启用 FTPS");
                builder = builder
                    .ftps(cert, key)
                    .ftps_required(require_ssl, require_ssl);
            } else {
                warn!("FTPS 已要求但证书/私钥文件不存在，以明文 FTP 运行");
            }
        }

        // 防爆破：连续失败锁定（用户+IP 维度）
        builder = builder.failed_logins_policy(libunftp::options::FailedLoginsPolicy::new(
            max_login_attempts,
            std::time::Duration::from_secs(ban_duration),
            libunftp::options::FailedLoginsBlock::UserAndIP,
        ));

        // 关停：oneshot 信号触发优雅关闭
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let (closed_tx, closed_rx) = tokio::sync::oneshot::channel::<()>();
        {
            *self.shutdown_tx.lock().await = Some(shutdown_tx);
        }
        {
            let mut slot = self.closed_rx.lock().unwrap();
            *slot = Some(closed_rx);
        }

        builder = builder.shutdown_indicator(async move {
            let _ = shutdown_rx.await;
            libunftp::options::Shutdown::new().grace_period(std::time::Duration::from_secs(5))
        });

        let server = builder.build()?;

        {
            let mut running = self.running.lock().unwrap();
            *running = true;
        }

        let bind_addr = format!("{bind_ip}:{port}");

        let listen_addr = bind_addr.clone();
        tokio::spawn(async move {
            if let Err(e) = server.listen(listen_addr).await {
                tracing::error!("FTP 服务异常退出: {e}");
            }
            let _ = closed_tx.send(());
        });

        info!(bind_addr = %bind_addr, "FTP 服务已启动 (libunftp)");
        Ok(())
    }

    pub async fn stop(&self) {
        if let Some(tx) = self.shutdown_tx.lock().await.take() {
            let _ = tx.send(());
        }

        let closed = self.closed_rx.lock().unwrap().take();
        if let Some(rx) = closed {
            let _ = tokio::time::timeout(std::time::Duration::from_secs(10), rx).await;
        }

        {
            let mut running = self.running.lock().unwrap();
            *running = false;
        }

        info!("FTP 服务已停止");
    }

    pub fn is_running(&self) -> bool {
        *self.running.lock().unwrap()
    }
}
