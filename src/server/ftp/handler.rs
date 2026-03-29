use anyhow::Result;
use rustls::ServerConfig;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::server::TlsStream;
use tracing::{warn, error};

use crate::core::config::Config;
use crate::core::file_logger::FileLogger;
use crate::core::users::UserManager;
use crate::server::common::login_tracker::LoginTracker;
use crate::server::common::quota::QuotaCache;

use super::data_connection::PassiveListenerMap;
use super::rate_limit::RateLimiter;
use super::tls::TlsConfig;
use super::utils::real_to_virtual_path;

#[derive(Default)]
pub enum FtpStream {
    Plain(TcpStream),
    Tls(Box<TlsStream<TcpStream>>),
    #[default]
    Taken,
}

impl FtpStream {
    pub async fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
        match self {
            FtpStream::Plain(stream) => stream.write_all(buf).await,
            FtpStream::Tls(stream) => stream.write_all(buf).await,
            FtpStream::Taken => Err(std::io::Error::new(std::io::ErrorKind::NotConnected, "Stream taken")),
        }
    }

    pub async fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            FtpStream::Plain(stream) => stream.read(buf).await,
            FtpStream::Tls(stream) => stream.read(buf).await,
            FtpStream::Taken => Err(std::io::Error::new(std::io::ErrorKind::NotConnected, "Stream taken")),
        }
    }

    pub fn peer_addr(&self) -> std::io::Result<std::net::SocketAddr> {
        match self {
            FtpStream::Plain(stream) => stream.peer_addr(),
            FtpStream::Tls(stream) => stream.get_ref().0.peer_addr(),
            FtpStream::Taken => Err(std::io::Error::new(std::io::ErrorKind::NotConnected, "Stream taken")),
        }
    }

    pub fn local_addr(&self) -> std::io::Result<std::net::SocketAddr> {
        match self {
            FtpStream::Plain(stream) => stream.local_addr(),
            FtpStream::Tls(stream) => stream.get_ref().0.local_addr(),
            FtpStream::Taken => Err(std::io::Error::new(std::io::ErrorKind::NotConnected, "Stream taken")),
        }
    }
}

pub struct FtpSessionConfig {
    pub config: Arc<std::sync::Mutex<Config>>,
    pub user_manager: Arc<std::sync::Mutex<UserManager>>,
    // pub logger: Arc<std::sync::Mutex<Logger>>,  // ← 已移除，使用 tracing
    pub file_logger: Arc<std::sync::Mutex<FileLogger>>,
    pub passive_listeners: PassiveListenerMap,
    pub rate_limiter: Arc<RateLimiter>,
    pub tls_config: Option<TlsConfig>,
    pub tls_server_config: Option<Arc<ServerConfig>>,
    pub login_tracker: Arc<LoginTracker>,
    pub quota_cache: Arc<QuotaCache>,
}

pub struct FtpSession {
    pub stream: FtpStream,
    pub config: Arc<std::sync::Mutex<Config>>,
    pub user_manager: Arc<std::sync::Mutex<UserManager>>,
    // pub logger: Arc<std::sync::Mutex<Logger>>,  // ← 已移除，使用 tracing
    pub file_logger: Arc<std::sync::Mutex<FileLogger>>,
    pub passive_listeners: PassiveListenerMap,
    pub rate_limiter: Arc<RateLimiter>,
    pub tls_config: Option<TlsConfig>,
    pub tls_server_config: Option<Arc<ServerConfig>>,
    pub login_tracker: Arc<LoginTracker>,
    pub quota_cache: Arc<QuotaCache>,
    pub remote_ip: String,
    pub local_ip: Option<String>,
    pub current_user: Option<String>,
    pub authenticated: bool,
    pub cwd: String,
    pub home_dir: String,
    pub data_port: Option<u16>,
    pub data_addr: Option<String>,
    pub passive_mode: bool,
    pub rest_offset: u64,
    pub rename_from: Option<String>,
    pub abort_flag: Arc<AtomicBool>,
    pub utf8_enabled: bool,
    pub binary_transfer: bool,
    pub tls_enabled: bool,
    pub tls_data_required: bool,
    pub pbsz_set: bool,
}

impl FtpSession {
    pub fn new(stream: TcpStream, session_config: FtpSessionConfig) -> Result<Self> {
        let remote_addr = stream.peer_addr()?;
        let remote_ip = remote_addr.ip().to_string();
        
        let local_ip = stream.local_addr()
            .ok()
            .map(|addr| addr.ip().to_string());

        Ok(Self {
            stream: FtpStream::Plain(stream),
            config: session_config.config,
            user_manager: session_config.user_manager,
            // logger: session_config.logger,  // ← 已移除
            file_logger: session_config.file_logger,
            passive_listeners: session_config.passive_listeners,
            rate_limiter: session_config.rate_limiter,
            tls_config: session_config.tls_config,
            tls_server_config: session_config.tls_server_config,
            login_tracker: session_config.login_tracker,
            quota_cache: session_config.quota_cache,
            remote_ip,
            local_ip,
            current_user: None,
            authenticated: false,
            cwd: String::new(),
            home_dir: String::new(),
            data_port: None,
            data_addr: None,
            passive_mode: false,
            rest_offset: 0,
            rename_from: None,
            abort_flag: Arc::new(AtomicBool::new(false)),
            utf8_enabled: true,
            binary_transfer: true,
            tls_enabled: false,
            tls_data_required: false,
            pbsz_set: false,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        if let Err(_e) = self.rate_limiter.check_and_record(&self.remote_ip) {
            warn!(client_ip = %self.remote_ip, "FTP 连接频率超限");
            self.stream.write_all(b"421 Too many connections, try again later\r\n").await?;
            return Ok(());
        }

        // 记录文件操作审计日志
        if let Ok(mut file_log) = self.file_logger.try_lock() {
            file_log.log(crate::core::file_logger::FileLogInfo {
                username: "anonymous",
                client_ip: &self.remote_ip,
                operation: "CONNECT",
                file_path: "-",
                file_size: 0,
                protocol: "FTP",
                success: true,
                message: "客户端已连接",
            });
        }

        let is_allowed = {
            let cfg = self.config.lock().unwrap();
            cfg.is_ip_allowed(&self.remote_ip)
        };
        
        if !is_allowed {
            warn!(client_ip = %self.remote_ip, "FTP 连接被 IP 过滤器拒绝");
            self.stream.write_all(b"530 Connection denied by IP filter\r\n").await?;
            self.rate_limiter.release_for_ip(&self.remote_ip);
            return Ok(());
        }

        let welcome_msg = {
            let cfg = self.config.lock().unwrap();
            cfg.ftp.welcome_message.clone()
        };
        self.stream.write_all(format!("220 {} \r\n", welcome_msg).as_bytes()).await?;

        let mut buffer = vec![0u8; 4096];

        loop {
            let idle_timeout = {
                let cfg = self.config.lock().unwrap();
                cfg.server.idle_timeout
            };
            
            let read_result = timeout(
                Duration::from_secs(idle_timeout),
                self.stream.read(&mut buffer)
            ).await;

            match read_result {
                Ok(Ok(0)) => break,
                Ok(Ok(n)) => {
                    let command = String::from_utf8_lossy(&buffer[..n])
                        .trim()
                        .to_string();

                    let parts: Vec<&str> = command.splitn(2, ' ').collect();
                    let cmd = parts[0].to_uppercase();
                    let arg = parts.get(1).map(|s| s.trim());

                    if self.handle_command(&cmd, arg).await? {
                        break;
                    }
                }
                Ok(Err(e)) => {
                    error!(client_ip = %self.remote_ip, error = %e, "FTP 读取错误");
                    break;
                }
                Err(_) => {
                    warn!(client_ip = %self.remote_ip, timeout = idle_timeout, "FTP 连接空闲超时");
                    break;
                }
            }
        }

        self.rate_limiter.release_for_ip(&self.remote_ip);
        Ok(())
    }

    async fn handle_command(&mut self, cmd: &str, arg: Option<&str>) -> Result<bool> {
        match cmd {
            "USER" => self.cmd_user(arg).await?,
            "PASS" => self.cmd_pass(arg).await?,
            "QUIT" => {
                self.stream.write_all(b"221 Goodbye\r\n").await?;
                return Ok(true);
            }
            "SYST" => self.stream.write_all(b"215 UNIX Type: L8\r\n").await?,
            "FEAT" => self.cmd_feat().await?,
            "HELP" => self.cmd_help(arg).await?,
            "MODE" => self.cmd_mode(arg).await?,
            "STRU" => self.cmd_stru(arg).await?,
            "ALLO" => self.stream.write_all(b"200 ALLO command successful\r\n").await?,
            "OPTS" => self.cmd_opts(arg).await?,
            "PWD" | "XPWD" => {
                let virtual_path = real_to_virtual_path(&self.cwd, &self.home_dir);
                self.stream.write_all(format!("257 \"{}\"\r\n", virtual_path).as_bytes()).await?
            }
            "CWD" => self.cmd_cwd(arg).await?,
            "CDUP" | "XCUP" => self.cmd_cdup().await?,
            "TYPE" => self.cmd_type(arg).await?,
            "MLST" => self.cmd_mlst(arg).await?,
            "REST" => self.cmd_rest(arg).await?,
            "PASV" => self.cmd_pasv().await?,
            "EPSV" => self.cmd_epsv().await?,
            "PORT" => self.cmd_port(arg).await?,
            "EPRT" => self.cmd_eprt(arg).await?,
            "AUTH" => self.cmd_auth(arg).await?,
            "PBSZ" => self.cmd_pbsz(arg).await?,
            "PROT" => self.cmd_prot(arg).await?,
            "LIST" | "NLST" => self.cmd_list(arg).await?,
            "MLSD" => self.cmd_mlsd().await?,
            "RETR" => self.cmd_retr(arg).await?,
            "STOR" => self.cmd_stor(arg).await?,
            "STOU" => self.cmd_stou(arg).await?,
            "APPE" => self.cmd_appe(arg).await?,
            "DELE" => self.cmd_dele(arg).await?,
            "MKD" | "XMKD" => self.cmd_mkd(arg).await?,
            "RMD" | "XRMD" => self.cmd_rmd(arg).await?,
            "RNFR" => self.cmd_rnfr(arg).await?,
            "RNTO" => self.cmd_rnto(arg).await?,
            "SIZE" => self.cmd_size(arg).await?,
            "MDTM" => self.cmd_mdtm(arg).await?,
            "NOOP" => self.stream.write_all(b"200 OK\r\n").await?,
            "STAT" => self.cmd_stat().await?,
            "ABOR" => self.handle_abor().await?,
            "REIN" => self.cmd_rein().await?,
            "SITE" => self.cmd_site(arg).await?,
            "CCC" => self.cmd_ccc().await?,
            _ => self.stream.write_all(b"502 Command not implemented\r\n").await?,
        }
        Ok(false)
    }

    pub async fn cmd_feat(&mut self) -> Result<()> {
        let mut features = vec![
            "211-Features:",
            " SIZE",
            " MDTM",
            " REST STREAM",
            " PASV",
            " EPSV",
            " EPRT",
            " PORT",
            " MLST",
            " MLSD",
            " MODE",
            " STRU",
            " UTF8",
            " TVFS",
            " STOU",
            " SITE",
        ];

        if self.tls_config.is_some() && self.tls_server_config.is_some() {
            features.push(" AUTH TLS");
            features.push(" AUTH SSL");
            features.push(" PBSZ");
            features.push(" PROT");
        }

        for feature in features {
            self.stream.write_all(feature.as_bytes()).await?;
            self.stream.write_all(b"\r\n").await?;
        }
        self.stream.write_all(b"211 End\r\n").await?;
        Ok(())
    }

    pub async fn cleanup_data_connection(&mut self) {
        if self.passive_mode
            && let Some(port) = self.data_port {
                let mut listeners = self.passive_listeners.lock().await;
                listeners.remove(&port);
            }
    }

    pub fn get_data_timeout(&self) -> u64 {
        let cfg = self.config.lock().unwrap();
        cfg.ftp.data_timeout
    }

    pub fn is_tls_available(&self) -> bool {
        self.tls_config.is_some() && self.tls_server_config.is_some()
    }

    pub fn is_tls_required(&self) -> bool {
        self.tls_config.as_ref().map(|c| c.require_tls).unwrap_or(false)
    }

    pub fn is_login_banned(&self) -> bool {
        self.login_tracker.is_banned(&self.remote_ip)
    }
}
