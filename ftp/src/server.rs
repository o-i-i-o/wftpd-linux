//! FtpServer：把 Config/UserManager/FileLogger 组装为 libunftp Server。
//!
//! 配置映射：
//! - `passive_ports` / `masquerade_ip` / `welcome_message` / `idle_timeout` → libunftp 选项
//! - `require_ssl` + cert/key → `FTPS（ftps_required`）
//! - `max_login_attempts` + `ban_duration` → libunftp FailedLoginsPolicy（防爆破锁定）
//! - `max_connections` → 连接数上限（在 accept 层强制）

use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};

use tracing::{info, warn};
use wftpd_common::server::quota::QuotaCache;
use wftpd_common::{Config, FileLogger, UserManager};

use crate::auth::{WftpdAuthenticator, WftpdUserDetailProvider};
use crate::storage::{WftpdFilesystem, WftpdUser};

/// `start` 时从 Config 快照出的启动参数
struct FtpStartOptions {
    bind_ip: String,
    port: u16,
    passive_ports: (u16, u16),
    masquerade_ip: Option<String>,
    greeting_leaked: &'static str,
    idle_timeout: u64,
    require_ssl: bool,
    cert_path: Option<String>,
    key_path: Option<String>,
    max_login_attempts: u32,
    ban_duration: u64,
}

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

    /// 启动 FTP 服务（已运行则直接返回）
    ///
    /// # Errors
    /// libunftp Server 构建失败时返回错误
    ///
    /// # Panics
    /// `running` / `closed_rx` 互斥锁中毒（持有线程 panic）时 panic
    pub async fn start(&self) -> anyhow::Result<()> {
        {
            let running = self.running.lock().unwrap();
            if *running {
                return Ok(());
            }
        }

        let options = self.lock_start_options();
        let mut builder = self.build_server_builder(&options);

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

        let bind_addr = format!("{}:{}", options.bind_ip, options.port);

        let listen_addr = bind_addr.clone();
        let running = Arc::clone(&self.running);
        tokio::spawn(async move {
            if let Err(e) = server.listen(listen_addr).await {
                tracing::error!("FTP 服务异常退出: {e}");
            }
            // 监听退出（含 bind 失败）后复位运行标志，保证状态查询如实上报
            *running.lock().unwrap() = false;
            let _ = closed_tx.send(());
        });

        info!(bind_addr = %bind_addr, "FTP 服务已启动 (libunftp)");
        Ok(())
    }

    /// 从配置读取启动参数；greeting 需 `&'static str`，泄漏一次可接受
    fn lock_start_options(&self) -> FtpStartOptions {
        let cfg = self.config.lock().unwrap();
        // libunftp 的 greeting 是 &'static str，配置来自启动时读取，泄漏一次可接受
        let greeting_leaked: &'static str =
            Box::leak(cfg.ftp.welcome_message.clone().into_boxed_str());
        FtpStartOptions {
            bind_ip: cfg.ftp.bind_ip.clone(),
            port: cfg.ftp.port,
            passive_ports: cfg.ftp.passive_ports,
            masquerade_ip: cfg.ftp.masquerade_ip.clone(),
            greeting_leaked,
            idle_timeout: cfg.security.idle_timeout,
            require_ssl: cfg.security.require_ssl,
            cert_path: cfg.security.cert_path.clone(),
            key_path: cfg.security.key_path.clone(),
            max_login_attempts: cfg.security.max_login_attempts,
            ban_duration: cfg.security.ban_duration,
        }
    }

    /// 组装 libunftp ServerBuilder（认证桥、存储后端、PASV/FTPS/防爆破选项）
    fn build_server_builder(
        &self,
        options: &FtpStartOptions,
    ) -> libunftp::ServerBuilder<WftpdFilesystem, WftpdUser> {
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
        .greeting(options.greeting_leaked)
        .passive_ports(options.passive_ports.0..=options.passive_ports.1)
        .idle_session_timeout(options.idle_timeout);

        builder = match options
            .masquerade_ip
            .as_deref()
            .and_then(|s| s.parse::<IpAddr>().ok())
            .and_then(|ip| match ip {
                IpAddr::V4(v4) => Some(v4),
                IpAddr::V6(_) => None,
            }) {
            Some(v4) => builder.passive_host(v4),
            None => builder,
        };

        // FTPS：证书可用即启用 AUTH TLS；require_ssl 时强制升级
        if let (Some(cert), Some(key)) = (&options.cert_path, &options.key_path) {
            let cert = PathBuf::from(cert);
            let key = PathBuf::from(key);
            if cert.exists() && key.exists() {
                info!(cert = %cert.display(), key = %key.display(), "启用 FTPS");
                builder = builder
                    .ftps(cert, key)
                    .ftps_required(options.require_ssl, options.require_ssl);
            } else {
                warn!("FTPS 已要求但证书/私钥文件不存在，以明文 FTP 运行");
            }
        }

        // 防爆破：连续失败锁定（用户+IP 维度）
        builder.failed_logins_policy(libunftp::options::FailedLoginsPolicy::new(
            options.max_login_attempts,
            std::time::Duration::from_secs(options.ban_duration),
            libunftp::options::FailedLoginsBlock::UserAndIP,
        ))
    }

    /// 停止服务并等待监听退出（最长 10 秒）
    ///
    /// # Panics
    /// `closed_rx` / `running` 互斥锁中毒（持有线程 panic）时 panic
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

    /// 服务是否处于运行状态
    ///
    /// # Panics
    /// `running` 互斥锁中毒（持有线程 panic）时 panic
    #[must_use]
    pub fn is_running(&self) -> bool {
        *self.running.lock().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wftpd_common::{Config, UserManager};

    fn make_server() -> FtpServer {
        FtpServer::new(
            Arc::new(StdMutex::new(Config::default())),
            Arc::new(StdMutex::new(UserManager::new())),
            Arc::new(StdMutex::new(wftpd_common::FileLogger::new("/tmp", 0))),
        )
    }

    #[test]
    fn new_server_is_not_running() {
        assert!(!make_server().is_running());
    }

    #[test]
    fn stop_before_start_is_noop() {
        let server = make_server();
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(server.stop());
        assert!(!server.is_running());
    }
}
