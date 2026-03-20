use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::core::config::Config;
use crate::core::logger::Logger;
use crate::core::users::UserManager;
use crate::core::file_logger::FileLogger;
use crate::core::quota::QuotaManager;
use crate::core::path_utils::{safe_resolve_path, to_ftp_path, resolve_directory_path, PathResolveError, path_starts_with_ignore_case};

use super::commands::FtpCommand;
use super::passive::PassiveManager;
use super::transfer;
use super::tls::{TlsConfig, AsyncTlsTcpStream};

const MAX_COMMAND_LENGTH: usize = 8192;

pub enum ControlStream {
    Plain(Option<TcpStream>),
    Tls(Box<AsyncTlsTcpStream>),
}

impl ControlStream {
    pub async fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            ControlStream::Plain(Some(stream)) => stream.read(buf).await,
            ControlStream::Plain(None) => Err(std::io::Error::new(std::io::ErrorKind::NotConnected, "No stream")),
            ControlStream::Tls(stream) => stream.read(buf).await,
        }
    }

    pub async fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
        match self {
            ControlStream::Plain(Some(stream)) => stream.write_all(buf).await,
            ControlStream::Plain(None) => Err(std::io::Error::new(std::io::ErrorKind::NotConnected, "No stream")),
            ControlStream::Tls(stream) => stream.write_all(buf).await,
        }
    }

    #[allow(dead_code)]
    pub fn is_tls(&self) -> bool {
        matches!(self, ControlStream::Tls(_))
    }

    pub async fn upgrade_to_tls(&mut self, acceptor: &tokio_native_tls::TlsAcceptor) -> Result<()> {
        if let ControlStream::Plain(stream_opt) = self
            && let Some(stream) = stream_opt.take() {
                let tls_stream = acceptor.accept(stream).await?;
                *self = ControlStream::Tls(Box::new(tls_stream));
            }
        Ok(())
    }
}

pub struct SessionState {
    pub current_user: Option<String>,
    pub authenticated: bool,
    pub cwd: String,
    pub home_dir: String,
    pub transfer_mode: String,
    pub passive_mode: bool,
    pub rest_offset: u64,
    pub rename_from: Option<String>,
    pub abort_flag: Arc<std::sync::atomic::AtomicBool>,
    pub passive_manager: PassiveManager,
    pub data_port: Option<u16>,
    pub data_addr: Option<String>,
    pub client_ip: String,
    pub tls_enabled: bool,
    pub data_protection: bool,
    pub pbsz_set: bool,
    pub file_structure: FileStructure,
    pub transfer_mode_type: TransferModeType,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FileStructure {
    File,
    Record,
    Page,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TransferModeType {
    Stream,
    Block,
    Compressed,
}

impl SessionState {
    pub fn new(client_ip: &str) -> Self {
        SessionState {
            current_user: None,
            authenticated: false,
            cwd: String::new(),
            home_dir: String::new(),
            transfer_mode: "binary".to_string(),
            passive_mode: true,
            rest_offset: 0,
            rename_from: None,
            abort_flag: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            passive_manager: PassiveManager::new(),
            data_port: None,
            data_addr: None,
            client_ip: client_ip.to_string(),
            tls_enabled: false,
            data_protection: false,
            pbsz_set: false,
            file_structure: FileStructure::File,
            transfer_mode_type: TransferModeType::Stream,
        }
    }

    pub fn resolve_path(&self, path: &str) -> Result<PathBuf, PathResolveError> {
        safe_resolve_path(&self.cwd, &self.home_dir, path)
    }

    #[allow(dead_code)]
    pub fn ftp_path(&self, local_path: &std::path::Path) -> Result<String, PathResolveError> {
        to_ftp_path(local_path, std::path::Path::new(&self.home_dir))
    }

    pub fn validate_port_ip(&self, port_ip: &str) -> bool {
        let parts: Vec<&str> = port_ip.split(',').collect();
        if parts.len() != 6 {
            return false;
        }
        
        let client_ip_parts: Vec<&str> = self.client_ip.split('.').collect();
        if client_ip_parts.len() != 4 {
            return true;
        }
        
        let port_ip_parts = &parts[0..4];
        for (i, part) in port_ip_parts.iter().enumerate() {
            if let Ok(num) = part.parse::<u8>()
                && let Ok(client_num) = client_ip_parts[i].parse::<u8>()
                && num != client_num {
                    log::warn!("PORT security: IP mismatch - expected {}, got {} in position {}", 
                        client_num, num, i);
                    return false;
                }
        }
        true
    }

    pub fn validate_eprt_ip(&self, net_addr: &str) -> bool {
        let client_ip: std::net::IpAddr = match self.client_ip.parse() {
            Ok(ip) => ip,
            Err(e) => {
                log::error!(
                    "EPRT security: Failed to parse client IP '{}': {}",
                    self.client_ip, e
                );
                return false;
            }
        };
        
        let eprt_ip: std::net::IpAddr = match net_addr.parse() {
            Ok(ip) => ip,
            Err(e) => {
                log::warn!(
                    "EPRT security: Failed to parse EPRT IP '{}': {}",
                    net_addr, e
                );
                return false;
            }
        };
        
        match (&client_ip, &eprt_ip) {
            (std::net::IpAddr::V4(client), std::net::IpAddr::V4(eprt)) => {
                if client != eprt {
                    log::warn!(
                        "EPRT security: IPv4 mismatch - expected {}, got {}",
                        client, eprt
                    );
                    return false;
                }
            }
            (std::net::IpAddr::V6(client), std::net::IpAddr::V6(eprt)) => {
                if client != eprt {
                    log::warn!(
                        "EPRT security: IPv6 mismatch - expected {}, got {}",
                        client, eprt
                    );
                    return false;
                }
            }
            _ => {
                log::warn!(
                    "EPRT security: IP version mismatch - client is {}, eprt is {}",
                    client_ip, eprt_ip
                );
                return false;
            }
        }
        
        true
    }
}

struct SessionConfig {
    welcome_msg: String,
    allow_anonymous: bool,
    anonymous_home: Option<String>,
    default_transfer_mode: String,
    default_passive_mode: bool,
    ip_allowed: bool,
    tls_config: TlsConfig,
    require_ssl: bool,
}

impl SessionConfig {
    fn from_config(config: &Config, client_ip: &str) -> Self {
        let tls_config = TlsConfig::new(
            config.security.cert_path.as_deref(),
            config.security.key_path.as_deref(),
            config.security.require_ssl,
        );
        
        SessionConfig {
            welcome_msg: config.ftp.welcome_message.clone(),
            allow_anonymous: config.ftp.allow_anonymous,
            anonymous_home: config.ftp.anonymous_home.clone(),
            default_transfer_mode: config.ftp.default_transfer_mode.clone(),
            default_passive_mode: config.ftp.default_passive_mode,
            ip_allowed: config.is_ip_allowed(client_ip),
            tls_config,
            require_ssl: config.security.require_ssl,
        }
    }
}

pub async fn handle_session(
    mut socket: TcpStream,
    config: Arc<std::sync::Mutex<Config>>,
    user_manager: Arc<std::sync::Mutex<UserManager>>,
    logger: Arc<std::sync::Mutex<Logger>>,
    file_logger: Arc<std::sync::Mutex<FileLogger>>,
    quota_manager: Arc<QuotaManager>,
    client_ip: String,
) -> Result<()> {
    let session_config = {
        let cfg = config.lock().map_err(|_| anyhow::anyhow!("Failed to lock config"))?;
        SessionConfig::from_config(&cfg, &client_ip)
    };

    if !session_config.ip_allowed {
        if let Ok(mut log) = logger.try_lock() {
            log.warning("FTP", &format!("Connection rejected from {} by IP filter", client_ip));
        }
        let _ = socket.write_all(b"530 Connection denied by IP filter\r\n").await;
        return Ok(());
    }

    let mut control_stream = ControlStream::Plain(Some(socket));
    let _ = control_stream.write_all(format!("220 {}\r\n", session_config.welcome_msg).as_bytes()).await;

    let mut state = SessionState::new(&client_ip);
    state.transfer_mode = session_config.default_transfer_mode;
    state.passive_mode = session_config.default_passive_mode;

    let mut cmd_buffer: Vec<u8> = Vec::with_capacity(MAX_COMMAND_LENGTH);
    let mut read_buffer = [0u8; 4096];

    loop {
        let conn_timeout = {
            let cfg = config.lock().map_err(|_| anyhow::anyhow!("Failed to lock config"))?;
            cfg.server.connection_timeout
        };

        let timeout_result = tokio::time::timeout(
            std::time::Duration::from_secs(conn_timeout),
            control_stream.read(&mut read_buffer)
        ).await;

        match timeout_result {
            Ok(Ok(0)) => break,
            Ok(Ok(n)) => {
                cmd_buffer.extend_from_slice(&read_buffer[..n]);
                
                if cmd_buffer.len() > MAX_COMMAND_LENGTH {
                    let _ = control_stream.write_all(b"500 Command too long\r\n").await;
                    cmd_buffer.clear();
                    continue;
                }

                while let Some(crlf_pos) = cmd_buffer.windows(2).position(|w| w == b"\r\n") {
                    let command_bytes: Vec<u8> = cmd_buffer.drain(..crlf_pos + 2).collect();
                    let command = String::from_utf8_lossy(&command_bytes[..command_bytes.len().saturating_sub(2)])
                        .trim()
                        .to_string();

                    let parts: Vec<&str> = command.splitn(2, ' ').collect();
                    let cmd = parts[0].to_uppercase();
                    let arg = parts.get(1).map(|s| s.trim());

                    let ftp_cmd = FtpCommand::parse(&cmd, arg);
                    
                    if !handle_command(
                        &mut control_stream,
                        &ftp_cmd,
                        &mut state,
                        &config,
                        &user_manager,
                        &logger,
                        &file_logger,
                        &quota_manager,
                        &client_ip,
                        &session_config.allow_anonymous,
                        &session_config.anonymous_home,
                        &session_config.tls_config,
                        session_config.require_ssl,
                    ).await? {
                        return Ok(());
                    }
                }
            }
            Ok(Err(e)) => {
                log::debug!("读取错误: {}", e);
                break;
            }
            Err(_) => {
                let _ = control_stream.write_all(b"421 Connection timed out\r\n").await;
                break;
            }
        }
    }

    Ok(())
}

pub async fn handle_session_tls(
    socket: AsyncTlsTcpStream,
    config: Arc<std::sync::Mutex<Config>>,
    user_manager: Arc<std::sync::Mutex<UserManager>>,
    logger: Arc<std::sync::Mutex<Logger>>,
    file_logger: Arc<std::sync::Mutex<FileLogger>>,
    quota_manager: Arc<QuotaManager>,
    client_ip: String,
) -> Result<()> {
    let session_config = {
        let cfg = config.lock().map_err(|_| anyhow::anyhow!("Failed to lock config"))?;
        SessionConfig::from_config(&cfg, &client_ip)
    };

    if !session_config.ip_allowed {
        if let Ok(mut log) = logger.try_lock() {
            log.warning("FTPS", &format!("Connection rejected from {} by IP filter", client_ip));
        }
        return Ok(());
    }

    let mut control_stream = ControlStream::Tls(Box::new(socket));
    let _ = control_stream.write_all(format!("220 {}\r\n", session_config.welcome_msg).as_bytes()).await;

    let mut state = SessionState::new(&client_ip);
    state.transfer_mode = session_config.default_transfer_mode;
    state.passive_mode = session_config.default_passive_mode;
    state.tls_enabled = true;

    let mut cmd_buffer: Vec<u8> = Vec::with_capacity(MAX_COMMAND_LENGTH);
    let mut read_buffer = [0u8; 4096];

    loop {
        let conn_timeout = {
            let cfg = config.lock().map_err(|_| anyhow::anyhow!("Failed to lock config"))?;
            cfg.server.connection_timeout
        };

        let timeout_result = tokio::time::timeout(
            std::time::Duration::from_secs(conn_timeout),
            control_stream.read(&mut read_buffer)
        ).await;

        match timeout_result {
            Ok(Ok(0)) => break,
            Ok(Ok(n)) => {
                cmd_buffer.extend_from_slice(&read_buffer[..n]);
                
                if cmd_buffer.len() > MAX_COMMAND_LENGTH {
                    let _ = control_stream.write_all(b"500 Command too long\r\n").await;
                    cmd_buffer.clear();
                    continue;
                }
                
                while let Some(pos) = cmd_buffer.iter().position(|&b| b == b'\n') {
                    let line: Vec<u8> = cmd_buffer.drain(..=pos).collect();
                    let line_str = String::from_utf8_lossy(&line);
                    let line_str = line_str.trim_end_matches('\r').trim_end_matches('\n');
                    
                    if line_str.is_empty() {
                        continue;
                    }
                    
                    let cmd = {
                        let parts: Vec<&str> = line_str.splitn(2, ' ').collect();
                        let cmd_str = parts[0].to_uppercase();
                        let arg = parts.get(1).map(|s| s.trim());
                        FtpCommand::parse(&cmd_str, arg)
                    };
                    
                    let (allow_anonymous, anonymous_home, tls_config, require_ssl) = {
                        let cfg = config.lock().map_err(|_| anyhow::anyhow!("Failed to lock config"))?;
                        (
                            cfg.ftp.allow_anonymous,
                            cfg.ftp.anonymous_home.clone(),
                            TlsConfig::new(
                                cfg.ftp.ftps.cert_path.as_deref(),
                                cfg.ftp.ftps.key_path.as_deref(),
                                cfg.ftp.ftps.require_ssl
                            ),
                            cfg.ftp.ftps.require_ssl
                        )
                    };
                    
                    let should_continue = handle_command(
                        &mut control_stream,
                        &cmd,
                        &mut state,
                        &config,
                        &user_manager,
                        &logger,
                        &file_logger,
                        &quota_manager,
                        &client_ip,
                        &allow_anonymous,
                        &anonymous_home,
                        &tls_config,
                        require_ssl,
                    ).await?;
                    
                    if !should_continue {
                        return Ok(());
                    }
                }
            }
            Ok(Err(_)) | Err(_) => {
                let _ = control_stream.write_all(b"421 Connection timed out\r\n").await;
                break;
            }
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn handle_command(
    control_stream: &mut ControlStream,
    cmd: &FtpCommand,
    state: &mut SessionState,
    config: &Arc<std::sync::Mutex<Config>>,
    user_manager: &Arc<std::sync::Mutex<UserManager>>,
    logger: &Arc<std::sync::Mutex<Logger>>,
    file_logger: &Arc<std::sync::Mutex<FileLogger>>,
    quota_manager: &Arc<QuotaManager>,
    client_ip: &str,
    allow_anonymous: &bool,
    anonymous_home: &Option<String>,
    tls_config: &TlsConfig,
    require_ssl: bool,
) -> Result<bool> {
    use super::commands::FtpCommand::*;

    match cmd {
        AUTH(tls_type) => {
            let tls_type = tls_type.as_deref().unwrap_or("TLS");
            let tls_upper = tls_type.to_uppercase();
            
            if tls_config.is_tls_available() {
                if tls_upper == "TLS" || tls_upper == "TLS-C" || tls_upper == "SSL" {
                    let _ = control_stream.write_all(b"234 AUTH command OK; starting TLS connection\r\n").await;
                    
                    if let Some(acceptor) = &tls_config.acceptor {
                        match control_stream.upgrade_to_tls(acceptor).await {
                            Ok(()) => {
                                state.tls_enabled = true;
                                log::info!("TLS connection established for {}", client_ip);
                            }
                            Err(e) => {
                                log::error!("TLS upgrade failed: {}", e);
                                let _ = control_stream.write_all(b"431 Unable to negotiate TLS connection\r\n").await;
                            }
                        }
                    }
                } else {
                    let _ = control_stream.write_all(format!("504 AUTH {} not supported\r\n", tls_type).as_bytes()).await;
                }
            } else {
                let _ = control_stream.write_all(b"502 TLS not configured on server\r\n").await;
            }
        }

        PBSZ(size) => {
            if state.tls_enabled {
                if let Some(size_str) = size {
                    if let Ok(size_val) = size_str.parse::<u64>() {
                        state.pbsz_set = true;
                        let _ = control_stream.write_all(format!("200 PBSZ={} OK\r\n", size_val).as_bytes()).await;
                    } else {
                        let _ = control_stream.write_all(b"501 Invalid PBSZ value\r\n").await;
                    }
                } else {
                    state.pbsz_set = true;
                    let _ = control_stream.write_all(b"200 PBSZ=0 OK\r\n").await;
                }
            } else {
                let _ = control_stream.write_all(b"503 PBSZ requires AUTH first\r\n").await;
            }
        }

        PROT(level) => {
            if state.tls_enabled && state.pbsz_set {
                if let Some(level) = level {
                    match level.to_uppercase().as_str() {
                        "P" => {
                            state.data_protection = true;
                            let _ = control_stream.write_all(b"200 PROT Private OK\r\n").await;
                        }
                        "C" => {
                            state.data_protection = false;
                            let _ = control_stream.write_all(b"200 PROT Clear OK\r\n").await;
                        }
                        "S" => {
                            let _ = control_stream.write_all(b"536 PROT Safe not supported\r\n").await;
                        }
                        "E" => {
                            let _ = control_stream.write_all(b"536 PROT Confidential not supported\r\n").await;
                        }
                        _ => {
                            let _ = control_stream.write_all(b"504 Unknown PROT level\r\n").await;
                        }
                    }
                } else {
                    let _ = control_stream.write_all(b"501 PROT requires parameter (C/P/S/E)\r\n").await;
                }
            } else {
                let _ = control_stream.write_all(b"503 PROT requires PBSZ first\r\n").await;
            }
        }

        CCC => {
            if state.tls_enabled {
                let _ = control_stream.write_all(b"200 CCC OK - reverting to clear text\r\n").await;
            } else {
                let _ = control_stream.write_all(b"533 CCC not available - not in TLS mode\r\n").await;
            }
        }

        USER(username) => {
            if require_ssl && !state.tls_enabled {
                let _ = control_stream.write_all(b"530 SSL required for login\r\n").await;
                return Ok(true);
            }
            
            let username_lower = username.to_lowercase();
            if username_lower == "anonymous" || username_lower == "ftp" {
                if *allow_anonymous {
                    state.current_user = Some("anonymous".to_string());
                    let _ = control_stream.write_all(b"331 Anonymous login okay, send email as password\r\n").await;
                } else {
                    let _ = control_stream.write_all(b"530 Anonymous access not allowed\r\n").await;
                }
            } else {
                state.current_user = Some(username.to_string());
                let _ = control_stream.write_all(b"331 User name okay, need password\r\n").await;
            }
        }

        PASS(password) => {
            if require_ssl && !state.tls_enabled {
                let _ = control_stream.write_all(b"530 SSL required for login\r\n").await;
                return Ok(true);
            }
            
            if let Some(ref username) = state.current_user {
                if username == "anonymous" {
                    if *allow_anonymous {
                        if let Some(anon_home) = anonymous_home {
                            match PathBuf::from(anon_home).canonicalize() {
                                Ok(home_canon) => {
                                    state.cwd = home_canon.to_string_lossy().to_string();
                                    state.home_dir = state.cwd.clone();
                                    state.authenticated = true;
                                    let _ = control_stream.write_all(b"230 Anonymous user logged in\r\n").await;
                                    if let Ok(mut logger_guard) = logger.lock() {
                                        logger_guard.client_action(
                                            "FTP",
                                            "Anonymous user logged in",
                                            client_ip,
                                            Some("anonymous"),
                                            "LOGIN",
                                        );
                                    }
                                }
                                Err(e) => {
                                    log::error!("PASS failed: cannot canonicalize anonymous home directory '{}': {}", anon_home, e);
                                    let _ = control_stream.write_all(b"550 Anonymous home directory not found\r\n").await;
                                    state.current_user = None;
                                }
                            }
                        } else {
                            log::error!("PASS failed: anonymous access allowed but no anonymous_home configured");
                            let _ = control_stream.write_all(b"530 Anonymous home directory not configured\r\n").await;
                            state.current_user = None;
                        }
                    } else {
                        let _ = control_stream.write_all(b"530 Anonymous access not allowed\r\n").await;
                    }
                } else {
                    let password = password.as_deref().unwrap_or("");
                    let (auth_result, home_dir_opt) = {
                        let mut users = user_manager.lock()
                            .map_err(|_| anyhow::anyhow!("Failed to lock user manager"))?;
                        if users.get_user(username).is_none() {
                            let _ = users.reload(&Config::get_users_path());
                        }
                        let result = users.authenticate(username, password);
                        let home = users.get_user(username).map(|u| u.home_dir.clone());
                        (result, home)
                    };

                    match auth_result {
                        Ok(true) => {
                            state.authenticated = true;
                            if let Some(home_dir) = home_dir_opt {
                                match PathBuf::from(&home_dir).canonicalize() {
                                    Ok(home_canon) => {
                                        state.cwd = home_canon.to_string_lossy().to_string();
                                        state.home_dir = state.cwd.clone();
                                    }
                                    Err(e) => {
                                        log::error!("PASS failed: cannot canonicalize user home directory '{}': {}", home_dir, e);
                                        let _ = control_stream.write_all(b"550 Home directory not found\r\n").await;
                                        state.authenticated = false;
                                        state.current_user = None;
                                        return Ok(true);
                                    }
                                }
                            }
                            let _ = control_stream.write_all(b"230 User logged in\r\n").await;
                            if let Ok(mut logger_guard) = logger.lock() {
                                logger_guard.client_action(
                                    "FTP",
                                    &format!("User {} logged in", username),
                                    client_ip,
                                    Some(username),
                                    "LOGIN",
                                );
                            }
                        }
                        Ok(false) => {
                            if let Ok(mut logger_guard) = logger.lock() {
                                logger_guard.client_action(
                                    "FTP",
                                    &format!("Authentication failed for user {}", username),
                                    client_ip,
                                    Some(username),
                                    "AUTH_FAIL",
                                );
                            }
                            let _ = control_stream.write_all(b"530 Not logged in, user cannot be authenticated\r\n").await;
                        }
                        Err(e) => {
                            if let Ok(mut logger_guard) = logger.lock() {
                                logger_guard.client_action(
                                    "FTP",
                                    &format!("Authentication error for user {}: {}", username, e),
                                    client_ip,
                                    Some(username),
                                    "AUTH_ERROR",
                                );
                            }
                            let _ = control_stream.write_all(b"530 Not logged in\r\n").await;
                        }
                    }
                }
            } else {
                let _ = control_stream.write_all(b"530 Please login with USER and PASS\r\n").await;
            }
        }

        QUIT => {
            let _ = control_stream.write_all(b"221 Goodbye\r\n").await;
            return Ok(false);
        }

        SYST => {
            let _ = control_stream.write_all(b"215 UNIX Type: L8\r\n").await;
        }

        FEAT => {
            let mut features = "211-Features:\r\n SIZE\r\n MDTM\r\n REST STREAM\r\n PASV\r\n EPSV\r\n EPRT\r\n PORT\r\n MLST\r\n MLSD\r\n MODE S\r\n STRU F\r\n UTF8\r\n TVFS\r\n".to_string();
            if tls_config.is_tls_available() {
                features.push_str(" AUTH TLS\r\n PBSZ\r\n PROT\r\n");
            }
            features.push_str("211 End\r\n");
            let _ = control_stream.write_all(features.as_bytes()).await;
        }

        NOOP => {
            let _ = control_stream.write_all(b"200 OK\r\n").await;
        }

        PWD | XPWD => {
            match to_ftp_path(std::path::Path::new(&state.cwd), std::path::Path::new(&state.home_dir)) {
                Ok(ftp_path) => {
                    let _ = control_stream.write_all(format!("257 \"{}\"\r\n", ftp_path).as_bytes()).await;
                }
                Err(e) => {
                    log::error!("PWD failed: {}", e);
                    let _ = control_stream.write_all(b"550 Failed to get current directory\r\n").await;
                }
            }
        }

        CWD(dir) => {
            if !state.authenticated {
                let _ = control_stream.write_all(b"530 Not logged in\r\n").await;
                return Ok(true);
            }
            if let Some(dir) = dir {
                match state.resolve_path(dir) {
                    Ok(new_path) => {
                        if new_path.exists() && new_path.is_dir() && path_starts_with_ignore_case(&new_path, &state.home_dir) {
                            state.cwd = new_path.to_string_lossy().to_string();
                            let _ = control_stream.write_all(b"250 Directory successfully changed\r\n").await;
                        } else {
                            let _ = control_stream.write_all(b"550 Failed to change directory\r\n").await;
                        }
                    }
                    Err(e) => {
                        log::warn!("CWD failed for '{}': {}", dir, e);
                        let _ = control_stream.write_all(format!("550 {}\r\n", e).as_bytes()).await;
                    }
                }
            } else {
                let _ = control_stream.write_all(b"501 Syntax error: CWD requires directory parameter\r\n").await;
            }
        }

        CDUP | XCUP => {
            if !state.authenticated {
                let _ = control_stream.write_all(b"530 Not logged in\r\n").await;
                return Ok(true);
            }
            match state.resolve_path("..") {
                Ok(new_path) => {
                    if path_starts_with_ignore_case(&new_path, &state.home_dir) && new_path.exists() {
                        state.cwd = new_path.to_string_lossy().to_string();
                        let _ = control_stream.write_all(b"250 Directory changed\r\n").await;
                    } else {
                        let _ = control_stream.write_all(b"550 Cannot change to parent directory: Permission denied\r\n").await;
                    }
                }
                Err(e) => {
                    log::warn!("CDUP failed: {}", e);
                    let _ = control_stream.write_all(format!("550 {}\r\n", e).as_bytes()).await;
                }
            }
        }

        TYPE(type_code) => {
            if let Some(type_code) = type_code {
                let type_upper = type_code.to_uppercase();
                let parts: Vec<&str> = type_upper.split_whitespace().collect();
                let main_type = parts.first().copied().unwrap_or("");
                let sub_type = parts.get(1).copied().unwrap_or("N");

                match main_type {
                    "I" => {
                        state.transfer_mode = "binary".to_string();
                        let _ = control_stream.write_all(b"200 Type set to I (Binary)\r\n").await;
                    }
                    "L" => {
                        if sub_type == "8" {
                            state.transfer_mode = "binary".to_string();
                            let _ = control_stream.write_all(b"200 Type set to L 8 (Local byte size 8)\r\n").await;
                        } else {
                            let _ = control_stream.write_all(b"504 Only L 8 is supported\r\n").await;
                        }
                    }
                    "A" => {
                        match sub_type {
                            "N" | "" => {
                                state.transfer_mode = "ascii".to_string();
                                let _ = control_stream.write_all(b"200 Type set to A (ASCII Non-print)\r\n").await;
                            }
                            "T" => {
                                state.transfer_mode = "ascii".to_string();
                                let _ = control_stream.write_all(b"200 Type set to A T (ASCII Telnet format)\r\n").await;
                            }
                            "C" => {
                                let _ = control_stream.write_all(b"504 ASA carriage control not supported\r\n").await;
                            }
                            _ => {
                                let _ = control_stream.write_all(b"501 Unknown subtype\r\n").await;
                            }
                        }
                    }
                    "E" => {
                        let _ = control_stream.write_all(b"504 EBCDIC not supported, use A or I\r\n").await;
                    }
                    _ => {
                        let _ = control_stream.write_all(b"501 Unknown type\r\n").await;
                    }
                }
            } else {
                if state.transfer_mode == "binary" {
                    let _ = control_stream.write_all(b"200 Type is I (Binary)\r\n").await;
                } else {
                    let _ = control_stream.write_all(b"200 Type is A (ASCII)\r\n").await;
                }
            }
        }

        MODE(mode) => {
            if let Some(mode) = mode {
                match mode.to_uppercase().as_str() {
                    "S" => {
                        state.transfer_mode_type = TransferModeType::Stream;
                        let _ = control_stream.write_all(b"200 Mode set to Stream\r\n").await;
                    }
                    "B" => {
                        state.transfer_mode_type = TransferModeType::Block;
                        let _ = control_stream.write_all(b"200 Mode set to Block\r\n").await;
                    }
                    "C" => {
                        state.transfer_mode_type = TransferModeType::Compressed;
                        let _ = control_stream.write_all(b"200 Mode set to Compressed\r\n").await;
                    }
                    _ => {
                        let _ = control_stream.write_all(b"501 Unknown mode\r\n").await;
                    }
                }
            } else {
                let _ = control_stream.write_all(b"501 Syntax error: MODE requires parameter\r\n").await;
            }
        }

        STRU(structure) => {
            if let Some(structure) = structure {
                match structure.to_uppercase().as_str() {
                    "F" => {
                        state.file_structure = FileStructure::File;
                        let _ = control_stream.write_all(b"200 Structure set to File\r\n").await;
                    }
                    "R" => {
                        state.file_structure = FileStructure::Record;
                        let _ = control_stream.write_all(b"200 Structure set to Record\r\n").await;
                    }
                    "P" => {
                        state.file_structure = FileStructure::Page;
                        let _ = control_stream.write_all(b"200 Structure set to Page\r\n").await;
                    }
                    _ => {
                        let _ = control_stream.write_all(b"501 Unknown structure\r\n").await;
                    }
                }
            } else {
                let _ = control_stream.write_all(b"501 Syntax error: STRU requires parameter\r\n").await;
            }
        }

        ALLO => {
            let _ = control_stream.write_all(b"200 ALLO command successful\r\n").await;
        }

        OPTS(opts_arg) => {
            if let Some(opts_arg) = opts_arg {
                let opts_upper = opts_arg.to_uppercase();
                if opts_upper.starts_with("UTF8") || opts_upper.starts_with("UTF-8") {
                    let _ = control_stream.write_all(b"200 UTF8 enabled\r\n").await;
                } else if opts_upper.starts_with("MODE") {
                    let _ = control_stream.write_all(b"200 Mode set\r\n").await;
                } else {
                    let _ = control_stream.write_all(b"200 Options set\r\n").await;
                }
            } else {
                let _ = control_stream.write_all(b"200 Options set\r\n").await;
            }
        }

        REST(offset_str) => {
            if let Some(offset_str) = offset_str {
                if let Ok(offset) = offset_str.parse::<u64>() {
                    state.rest_offset = offset;
                    let _ = control_stream.write_all(format!("350 Restarting at {}\r\n", offset).as_bytes()).await;
                    if let Ok(mut log) = logger.lock() {
                        log.client_action(
                            "FTP",
                            &format!("REST command: offset {}", offset),
                            client_ip,
                            state.current_user.as_deref(),
                            "REST",
                        );
                    }
                } else {
                    let _ = control_stream.write_all(b"501 Syntax error in REST parameter\r\n").await;
                }
            } else {
                state.rest_offset = 0;
                let _ = control_stream.write_all(b"350 Restarting at 0\r\n").await;
            }
        }

        PASV => {
            let ((port_min, port_max), bind_ip) = {
                let cfg = config.lock()
                    .map_err(|_| anyhow::anyhow!("Failed to lock config"))?;
                (cfg.ftp.passive_ports, cfg.ftp.bind_ip.clone())
            };

            let passive_port = match state.passive_manager.try_bind_port(port_min, port_max, &bind_ip).await {
                Ok(port) => port,
                Err(e) => {
                    let _ = control_stream.write_all(format!("425 Could not enter passive mode: {}\r\n", e).as_bytes()).await;
                    return Ok(true);
                }
            };

            state.passive_mode = true;
            state.data_port = Some(passive_port);

            let response_ip = if bind_ip == "0.0.0.0" || bind_ip.is_empty() {
                client_ip.to_string()
            } else {
                bind_ip.clone()
            };

            let ip_parts: Vec<&str> = response_ip.split('.').collect();
            if ip_parts.len() != 4 {
                let _ = control_stream.write_all(b"425 Invalid IP address format\r\n").await;
                return Ok(true);
            }

            let p1 = passive_port >> 8;
            let p2 = passive_port & 0xFF;

            let _ = control_stream.write_all(
                format!(
                    "227 Entering Passive Mode ({},{},{},{},{},{}).\r\n",
                    ip_parts[0], ip_parts[1], ip_parts[2], ip_parts[3], p1, p2
                )
                .as_bytes(),
            ).await;

            if let Ok(mut log) = logger.lock() {
                log.client_action(
                    "FTP",
                    &format!("PASV mode: port {}", passive_port),
                    client_ip,
                    state.current_user.as_deref(),
                    "PASV",
                );
            }
        }

        EPSV => {
            let ((port_min, port_max), bind_ip) = {
                let cfg = config.lock()
                    .map_err(|_| anyhow::anyhow!("Failed to lock config"))?;
                (cfg.ftp.passive_ports, cfg.ftp.bind_ip.clone())
            };

            let passive_port = match state.passive_manager.try_bind_port(port_min, port_max, &bind_ip).await {
                Ok(port) => port,
                Err(e) => {
                    let _ = control_stream.write_all(format!("425 Could not enter extended passive mode: {}\r\n", e).as_bytes()).await;
                    return Ok(true);
                }
            };

            state.passive_mode = true;
            state.data_port = Some(passive_port);

            let _ = control_stream.write_all(
                format!("229 Entering Extended Passive Mode (|||{}|)\r\n", passive_port).as_bytes(),
            ).await;
        }

        PORT(data) => {
            if let Some(data) = data {
                let parts: Vec<u16> = data.split(',').filter_map(|s| s.parse().ok()).collect();
                if parts.len() == 6 {
                    if !state.validate_port_ip(data) {
                        let _ = control_stream.write_all(b"500 PORT command rejected: IP address must match control connection\r\n").await;
                        return Ok(true);
                    }
                    
                    let port = parts[4] * 256 + parts[5];
                    let addr = format!("{}.{}.{}.{}:{}", parts[0], parts[1], parts[2], parts[3], port);
                    state.data_port = Some(port);
                    state.data_addr = Some(addr);
                    state.passive_mode = false;
                    let _ = control_stream.write_all(b"200 PORT command successful\r\n").await;
                } else {
                    let _ = control_stream.write_all(b"501 Syntax error in parameters or arguments\r\n").await;
                }
            } else {
                let _ = control_stream.write_all(b"501 Syntax error: PORT requires parameters\r\n").await;
            }
        }

        EPRT(data) => {
            if let Some(data) = data {
                let parts: Vec<&str> = data.split('|').collect();
                if parts.len() >= 4 {
                    let net_proto = parts[1];
                    let net_addr = parts[2];
                    let tcp_port = parts[3];

                    match net_proto {
                        "1" => {
                            if let Ok(port) = tcp_port.parse::<u16>() {
                                if !state.validate_eprt_ip(net_addr) {
                                    let _ = control_stream.write_all(b"500 EPRT command rejected: IP address must match control connection\r\n").await;
                                    return Ok(true);
                                }
                                state.data_port = Some(port);
                                state.data_addr = Some(format!("{}:{}", net_addr, port));
                                state.passive_mode = false;
                                let _ = control_stream.write_all(b"200 EPRT command successful\r\n").await;
                            } else {
                                let _ = control_stream.write_all(b"501 Invalid port number\r\n").await;
                            }
                        }
                        "2" => {
                            if let Ok(port) = tcp_port.parse::<u16>() {
                                if !state.validate_eprt_ip(net_addr) {
                                    let _ = control_stream.write_all(b"500 EPRT command rejected: IP address must match control connection\r\n").await;
                                    return Ok(true);
                                }
                                state.data_port = Some(port);
                                state.data_addr = Some(format!("[{}]:{}", net_addr, port));
                                state.passive_mode = false;
                                let _ = control_stream.write_all(b"200 EPRT command successful (IPv6)\r\n").await;
                            } else {
                                let _ = control_stream.write_all(b"501 Invalid port number\r\n").await;
                            }
                        }
                        _ => {
                            let _ = control_stream.write_all(b"522 Protocol not supported, use (1,2)\r\n").await;
                        }
                    }
                } else {
                    let _ = control_stream.write_all(b"501 Syntax error in EPRT parameters\r\n").await;
                }
            } else {
                let _ = control_stream.write_all(b"501 Syntax error: EPRT requires parameters\r\n").await;
            }
        }

        ABOR => {
            state.abort_flag.store(true, std::sync::atomic::Ordering::Relaxed);
            let _ = control_stream.write_all(b"426 Connection closed; transfer aborted\r\n").await;
            let _ = control_stream.write_all(b"226 Abort successful\r\n").await;
        }

        REIN => {
            state.authenticated = false;
            state.current_user = None;
            state.cwd = String::new();
            state.home_dir = String::new();
            state.data_port = None;
            state.data_addr = None;
            state.rest_offset = 0;
            state.rename_from = None;
            state.data_protection = false;
            state.pbsz_set = false;
            let _ = control_stream.write_all(b"220 Service ready for new user\r\n").await;
        }

        ACCT => {
            let _ = control_stream.write_all(b"202 Account not required\r\n").await;
        }

        HELP(cmd) => {
            if let Some(cmd) = cmd {
                let help_text = match cmd.to_uppercase().as_str() {
                    "USER" => "214 USER <username>: Specify user name\r\n",
                    "PASS" => "214 PASS <password>: Specify password\r\n",
                    "CWD" => "214 CWD <directory>: Change working directory\r\n",
                    "CDUP" => "214 CDUP: Change to parent directory\r\n",
                    "PWD" => "214 PWD: Print working directory\r\n",
                    "LIST" => "214 LIST [<path>]: List directory contents\r\n",
                    "NLST" => "214 NLST [<path>]: List directory names\r\n",
                    "RETR" => "214 RETR <filename>: Retrieve file\r\n",
                    "STOR" => "214 STOR <filename>: Store file\r\n",
                    "DELE" => "214 DELE <filename>: Delete file\r\n",
                    "MKD" => "214 MKD <directory>: Create directory\r\n",
                    "RMD" => "214 RMD <directory>: Remove directory\r\n",
                    "RNFR" => "214 RNFR <filename>: Specify rename source\r\n",
                    "RNTO" => "214 RNTO <filename>: Specify rename destination\r\n",
                    "PASV" => "214 PASV: Enter passive mode\r\n",
                    "EPSV" => "214 EPSV: Enter extended passive mode\r\n",
                    "PORT" => "214 PORT <h1,h2,h3,h4,p1,p2>: Enter active mode\r\n",
                    "EPRT" => "214 EPRT |<netproto>|<netaddr>|<tcpport>|: Extended active mode\r\n",
                    "TYPE" => "214 TYPE <type>: Set transfer type (A/I)\r\n",
                    "MODE" => "214 MODE <mode>: Set transfer mode (S/B/C)\r\n",
                    "STRU" => "214 STRU <structure>: Set file structure (F/R/P)\r\n",
                    "REST" => "214 REST <offset>: Set restart marker\r\n",
                    "SIZE" => "214 SIZE <filename>: Get file size\r\n",
                    "MDTM" => "214 MDTM <filename>: Get modification time\r\n",
                    "ABOR" => "214 ABOR: Abort current transfer\r\n",
                    "QUIT" => "214 QUIT: Disconnect from server\r\n",
                    "AUTH" => "214 AUTH <type>: Initiate TLS (TLS/SSL)\r\n",
                    "PBSZ" => "214 PBSZ <size>: Set protection buffer size\r\n",
                    "PROT" => "214 PROT <level>: Set data protection (C/P)\r\n",
                    _ => "214 Unknown command\r\n",
                };
                let _ = control_stream.write_all(help_text.as_bytes()).await;
            } else {
                let _ = control_stream.write_all(b"214-The following commands are recognized:\r\n").await;
                let _ = control_stream.write_all(b"214-USER PASS ACCT CWD CDUP PWD LIST NLST RETR STOR\r\n").await;
                let _ = control_stream.write_all(b"214-DELE MKD RMD RNFR RNTO PASV EPSV PORT EPRT\r\n").await;
                let _ = control_stream.write_all(b"214-TYPE MODE STRU REST SIZE MDTM ABOR QUIT REIN\r\n").await;
                let _ = control_stream.write_all(b"214-MLSD MLST SYST FEAT STAT HELP NOOP STOU SITE\r\n").await;
                if tls_config.is_tls_available() {
                    let _ = control_stream.write_all(b"214-AUTH PBSZ PROT CCC\r\n").await;
                }
                let _ = control_stream.write_all(b"214 Direct comments to admin\r\n").await;
            }
        }

        STAT => {
            if let Some(ref username) = state.current_user {
                let _ = control_stream.write_all(b"211-FTP server status:\r\n").await;
                let _ = control_stream.write_all(format!("211-Connected to: {}\r\n", client_ip).as_bytes()).await;
                let _ = control_stream.write_all(format!("211-Logged in as: {}\r\n", username).as_bytes()).await;
                let _ = control_stream.write_all(format!("211-Current directory: {}\r\n", state.cwd).as_bytes()).await;
                let _ = control_stream.write_all(format!("211-Transfer mode: {}\r\n", if state.passive_mode { "Passive" } else { "Active" }).as_bytes()).await;
                let _ = control_stream.write_all(format!("211-TLS: {}\r\n", if state.tls_enabled { "Enabled" } else { "Disabled" }).as_bytes()).await;
                let _ = control_stream.write_all(b"211 End\r\n").await;
            } else {
                let _ = control_stream.write_all(b"211 FTP server status - Not logged in\r\n").await;
            }
        }

        SITE(cmd) => {
            if let Some(site_cmd) = cmd {
                let site_parts: Vec<&str> = site_cmd.splitn(2, ' ').collect();
                let site_action = site_parts[0].to_uppercase();
                let site_arg = site_parts.get(1).map(|s| s.trim());

                match site_action.as_str() {
                    "HELP" => {
                        let _ = control_stream.write_all(b"214-The following SITE commands are recognized:\r\n").await;
                        let _ = control_stream.write_all(b"214-CHMOD IDLE HELP\r\n").await;
                        let _ = control_stream.write_all(b"214 End\r\n").await;
                    }
                    "IDLE" => {
                        if let Some(secs_str) = site_arg {
                            if let Ok(secs) = secs_str.parse::<u64>() {
                                let _ = control_stream.write_all(format!("200 Idle timeout set to {} seconds\r\n", secs).as_bytes()).await;
                            } else {
                                let _ = control_stream.write_all(b"501 Invalid idle time\r\n").await;
                            }
                        } else {
                            let _ = control_stream.write_all(b"501 SITE IDLE requires time parameter\r\n").await;
                        }
                    }
                    "CHMOD" => {
                        if !state.authenticated {
                            let _ = control_stream.write_all(b"530 Not logged in\r\n").await;
                            return Ok(true);
                        }
                        
                        if let Some(chmod_args) = site_arg {
                            let chmod_parts: Vec<&str> = chmod_args.splitn(2, ' ').collect();
                            if chmod_parts.len() == 2 {
                                let mode = chmod_parts[0];
                                let target = chmod_parts[1];
                                
                                let target_path = match state.resolve_path(target) {
                                    Ok(p) => p,
                                    Err(e) => {
                                        let _ = control_stream.write_all(format!("550 {}\r\n", e).as_bytes()).await;
                                        return Ok(true);
                                    }
                                };
                                
                                if !path_starts_with_ignore_case(&target_path, &state.home_dir) {
                                    let _ = control_stream.write_all(b"550 Permission denied\r\n").await;
                                    return Ok(true);
                                }
                                
                                if let Ok(_mode_val) = u32::from_str_radix(mode, 8) {
                                    #[cfg(windows)]
                                    {
                                        let _ = control_stream.write_all(b"200 CHMOD command accepted (Windows: permissions managed by ACL)\r\n").await;
                                    }
                                    #[cfg(not(windows))]
                                    {
                                        use std::os::unix::fs::PermissionsExt;
                                        match std::fs::set_permissions(&target_path, std::fs::Permissions::from_mode(mode_val)) {
                                            Ok(()) => {
                                                let _ = control_stream.write_all(format!("200 CHMOD {} {}\r\n", mode, target).as_bytes()).await;
                                            }
                                            Err(e) => {
                                                let _ = control_stream.write_all(format!("550 CHMOD failed: {}\r\n", e).as_bytes()).await;
                                            }
                                        }
                                    }
                                } else {
                                    let _ = control_stream.write_all(b"501 Invalid mode format\r\n").await;
                                }
                            } else {
                                let _ = control_stream.write_all(b"501 SITE CHMOD requires mode and filename\r\n").await;
                            }
                        } else {
                            let _ = control_stream.write_all(b"501 SITE CHMOD requires parameters\r\n").await;
                        }
                    }
                    _ => {
                        let _ = control_stream.write_all(format!("500 Unknown SITE command: {}\r\n", site_action).as_bytes()).await;
                    }
                }
            } else {
                let _ = control_stream.write_all(b"501 SITE command requires parameter\r\n").await;
            }
        }

        LIST(path) | NLST(path) => {
            if !state.authenticated {
                let _ = control_stream.write_all(b"530 Not logged in\r\n").await;
                return Ok(true);
            }

            let can_list = if state.current_user.as_deref() == Some("anonymous") {
                *allow_anonymous
            } else {
                let users = user_manager.lock()
                    .map_err(|_| anyhow::anyhow!("Failed to lock user manager"))?;
                let user = state.current_user.as_ref().and_then(|u| users.get_user(u));
                user.is_some_and(|u| u.permissions.can_list)
            };

            if !can_list {
                let _ = control_stream.write_all(b"550 Permission denied\r\n").await;
                return Ok(true);
            }

            let list_path = if let Some(path_arg) = path {
                match resolve_directory_path(&state.cwd, &state.home_dir, path_arg) {
                    Ok(path) => path,
                    Err(PathResolveError::PathEscape) => {
                        let _ = control_stream.write_all(b"550 Permission denied\r\n").await;
                        return Ok(true);
                    }
                    Err(PathResolveError::NotADirectory) => {
                        let _ = control_stream.write_all(b"550 Not a directory\r\n").await;
                        return Ok(true);
                    }
                    Err(PathResolveError::NotFound) => {
                        let _ = control_stream.write_all(b"550 Directory not found\r\n").await;
                        return Ok(true);
                    }
                    Err(_) => {
                        let _ = control_stream.write_all(b"550 Failed to resolve path\r\n").await;
                        return Ok(true);
                    }
                }
            } else {
                PathBuf::from(&state.cwd)
            };

            let _ = control_stream.write_all(b"150 Here comes the directory listing\r\n").await;

            let current_username = state.current_user.clone().unwrap_or_else(|| "anonymous".to_string());
            let is_ascii = state.transfer_mode == "ascii";
            
            if let Ok(mut data_stream) = transfer::get_data_connection(
                state.passive_mode,
                state.data_port,
                &state.data_addr,
                client_ip,
                &mut state.passive_manager,
            ).await {
                let is_nlst = matches!(cmd, NLST(_));
                if let Err(e) = transfer::send_directory_listing(
                    &mut data_stream,
                    &list_path,
                    &current_username,
                    is_nlst,
                    is_ascii,
                ).await {
                    log::warn!("LIST/NLST transfer error: {}", e);
                }
            }

            if state.passive_mode
                && let Some(port) = state.data_port {
                    state.passive_manager.remove_listener(port);
                }

            let _ = control_stream.write_all(b"226 Transfer complete\r\n").await;
        }

        MLSD(path) | MLST(path) => {
            if !state.authenticated {
                let _ = control_stream.write_all(b"530 Not logged in\r\n").await;
                return Ok(true);
            }

            let can_list = {
                let users = user_manager.lock()
                    .map_err(|_| anyhow::anyhow!("Failed to lock user manager"))?;
                let user = state.current_user.as_ref().and_then(|u| users.get_user(u));
                user.is_none_or(|u| u.permissions.can_list)
            };

            if !can_list {
                let _ = control_stream.write_all(b"550 Permission denied\r\n").await;
                return Ok(true);
            }

            let target_path = if let Some(path_arg) = path {
                match resolve_directory_path(&state.cwd, &state.home_dir, path_arg) {
                    Ok(path) => path,
                    Err(_) => {
                        let _ = control_stream.write_all(b"550 Failed to resolve path\r\n").await;
                        return Ok(true);
                    }
                }
            } else {
                PathBuf::from(&state.cwd)
            };

            if matches!(cmd, MLST(_)) {
                if target_path.exists() && path_starts_with_ignore_case(&target_path, &state.home_dir) {
                    if let Ok(metadata) = tokio::fs::metadata(&target_path).await {
                        let owner = state.current_user.as_deref().unwrap_or("anonymous");
                        let facts = transfer::build_mlst_facts(&metadata, owner);
                        match to_ftp_path(&target_path, std::path::Path::new(&state.home_dir)) {
                            Ok(ftp_path) => {
                                let name = target_path.file_name()
                                    .map(|n| n.to_string_lossy().to_string())
                                    .unwrap_or_else(|| target_path.to_string_lossy().to_string());
                                let _ = control_stream.write_all(format!("250-Listing {}\r\n {} {}\r\n250 End\r\n", ftp_path, facts, name).as_bytes()).await;
                            }
                            Err(e) => {
                                log::error!("MLST failed: {}", e);
                                let _ = control_stream.write_all(b"550 Failed to get file path\r\n").await;
                            }
                        }
                    } else {
                        let _ = control_stream.write_all(b"550 Failed to get file info\r\n").await;
                    }
                } else {
                    let _ = control_stream.write_all(b"550 File not found\r\n").await;
                }
            } else {
                let _ = control_stream.write_all(b"150 Here comes the directory listing\r\n").await;

                let mlst_owner = state.current_user.clone().unwrap_or_else(|| "anonymous".to_string());
                
                if let Ok(mut data_stream) = transfer::get_data_connection(
                    state.passive_mode,
                    state.data_port,
                    &state.data_addr,
                    client_ip,
                    &mut state.passive_manager,
                ).await
                    && let Err(e) = transfer::send_mlsd_listing(
                        &mut data_stream,
                        &target_path,
                        &mlst_owner,
                    ).await {
                        log::warn!("MLSD transfer error: {}", e);
                    }

                if state.passive_mode
                    && let Some(port) = state.data_port {
                        state.passive_manager.remove_listener(port);
                    }

                let _ = control_stream.write_all(b"226 Transfer complete\r\n").await;
            }
        }

        RETR(filename) => {
            if !state.authenticated {
                let _ = control_stream.write_all(b"530 Not logged in\r\n").await;
                return Ok(true);
            }

            if let Some(filename) = filename {
                let file_path = match state.resolve_path(filename) {
                    Ok(p) => p,
                    Err(e) => {
                        log::warn!("RETR failed for '{}': {}", filename, e);
                        let _ = control_stream.write_all(format!("550 {}\r\n", e).as_bytes()).await;
                        return Ok(true);
                    }
                };

                let normalized_home_dir = state.home_dir.replace('/', "\\");
                let normalized_file_path_str = file_path.to_string_lossy().replace('/', "\\");
                let starts_with_home = path_starts_with_ignore_case(&file_path, &state.home_dir) || normalized_file_path_str.to_lowercase().starts_with(&normalized_home_dir.to_lowercase());

                if !file_path.exists() || !file_path.is_file() || !starts_with_home {
                    log::warn!("RETR denied: path='{}', home='{}', exists={}, is_file={}, starts_with={}", 
                        file_path.display(), state.home_dir, file_path.exists(), file_path.is_file(), starts_with_home);
                    let _ = control_stream.write_all(b"550 File not found\r\n").await;
                    return Ok(true);
                }

                let (can_read, speed_limit_kbps) = {
                    let users = user_manager.lock()
                        .map_err(|_| anyhow::anyhow!("Failed to lock user manager"))?;
                    let user = state.current_user.as_ref().and_then(|u| users.get_user(u));
                    (
                        user.is_none_or(|u| u.permissions.can_read),
                        user.and_then(|u| u.permissions.speed_limit_kbps)
                    )
                };

                if !can_read {
                    let _ = control_stream.write_all(b"550 Permission denied\r\n").await;
                    return Ok(true);
                }

                let file_metadata = match tokio::fs::metadata(&file_path).await {
                    Ok(m) => m,
                    Err(e) => {
                        let _ = control_stream.write_all(format!("450 File unavailable: {}\r\n", e).as_bytes()).await;
                        return Ok(true);
                    }
                };
                
                let file_size = file_metadata.len();
                let remaining = if state.rest_offset > 0 && state.rest_offset < file_size {
                    file_size - state.rest_offset
                } else {
                    file_size
                };

                if state.rest_offset > 0 {
                    let _ = control_stream.write_all(format!("110 Restart marker at {}\r\n", state.rest_offset).as_bytes()).await;
                }

                let _ = control_stream.write_all(
                    format!("150 Opening BINARY mode data connection ({} bytes)\r\n", remaining)
                        .as_bytes(),
                ).await;

                let is_ascii = state.transfer_mode == "ascii";
                let rate_limiter = speed_limit_kbps.map(crate::core::rate_limiter::RateLimiter::new);
                
                if let Ok(mut data_stream) = transfer::get_data_connection(
                    state.passive_mode,
                    state.data_port,
                    &state.data_addr,
                    client_ip,
                    &mut state.passive_manager,
                ).await {
                    let abort = Arc::clone(&state.abort_flag);
                    if let Err(e) = transfer::send_file_with_limits(&mut data_stream, &file_path, state.rest_offset, abort, is_ascii, rate_limiter.as_ref()).await {
                        log::warn!("RETR transfer error: {}", e);
                    }
                }

                if state.passive_mode
                    && let Some(port) = state.data_port {
                        state.passive_manager.remove_listener(port);
                    }

                let _ = control_stream.write_all(b"226 Transfer complete\r\n").await;

                let final_size = tokio::fs::metadata(&file_path).await.map(|m| m.len()).unwrap_or(remaining);
                if let Ok(mut fl) = file_logger.lock() {
                    fl.log_download(
                        state.current_user.as_deref().unwrap_or("anonymous"),
                        client_ip,
                        &file_path.to_string_lossy(),
                        final_size,
                        "FTP",
                    );
                }

                if let Ok(mut log) = logger.lock() {
                    log.client_action(
                        "FTP",
                        &format!(
                            "Downloaded: {} ({} bytes from offset {})",
                            filename, remaining, state.rest_offset
                        ),
                        client_ip,
                        state.current_user.as_deref(),
                        "DOWNLOAD",
                    );
                }

                state.rest_offset = 0;
            }
        }

        STOR(filename) => {
            if !state.authenticated {
                let _ = control_stream.write_all(b"530 Not logged in\r\n").await;
                return Ok(true);
            }

            if let Some(filename) = filename {
                let (can_write, quota_mb, speed_limit_kbps) = {
                    let users = user_manager.lock()
                        .map_err(|_| anyhow::anyhow!("Failed to lock user manager"))?;
                    let user = state.current_user.as_ref().and_then(|u| users.get_user(u));
                    (
                        user.is_some_and(|u| u.permissions.can_write),
                        user.and_then(|u| u.permissions.quota_mb),
                        user.and_then(|u| u.permissions.speed_limit_kbps),
                    )
                };

                if !can_write {
                    log::warn!("STOR denied: user {} lacks write permission", state.current_user.as_deref().unwrap_or("unknown"));
                    let _ = control_stream.write_all(b"550 Permission denied\r\n").await;
                    return Ok(true);
                }

                let is_abs = filename.starts_with('/');
                log::info!("STOR: raw_filename='{}', is_absolute={}, cwd='{}', home='{}', passive_mode={}, data_port={:?}", 
                    filename, is_abs, state.cwd, state.home_dir, state.passive_mode, state.data_port);
                
                let file_path = match state.resolve_path(filename) {
                    Ok(p) => {
                        log::info!("STOR: resolved_path='{}'", p.display());
                        p
                    },
                    Err(e) => {
                        log::warn!("STOR failed for '{}': {}", filename, e);
                        let _ = control_stream.write_all(format!("550 {}\r\n", e).as_bytes()).await;
                        return Ok(true);
                    }
                };
                
                let normalized_home_dir = state.home_dir.replace('/', "\\");
                let normalized_file_path_str = file_path.to_string_lossy().replace('/', "\\");
                let starts_with_home = path_starts_with_ignore_case(&file_path, &state.home_dir) || normalized_file_path_str.to_lowercase().starts_with(&normalized_home_dir.to_lowercase());
                
                log::info!("STOR: resolved='{}', normalized_home='{}', starts_with={}", 
                    file_path.display(), normalized_home_dir, starts_with_home);
                if !starts_with_home {
                    log::warn!("STOR denied: path outside home - {} (home: {})", file_path.display(), state.home_dir);
                    let _ = control_stream.write_all(b"550 Permission denied\r\n").await;
                    return Ok(true);
                }

                if let Some(quota) = quota_mb {
                    let current_usage = quota_manager.get_usage(state.current_user.as_deref().unwrap_or("anonymous")).await;
                    let quota_bytes = quota * 1024 * 1024;
                    if current_usage >= quota_bytes {
                        let _ = control_stream.write_all(b"552 Quota exceeded\r\n").await;
                        if let Ok(mut log) = logger.lock() {
                            log.client_action(
                                "FTP",
                                &format!("Upload denied: quota exceeded for user {}", state.current_user.as_deref().unwrap_or("unknown")),
                                client_ip,
                                state.current_user.as_deref(),
                                "QUOTA_EXCEEDED",
                            );
                        }
                        return Ok(true);
                    }
                }

                let file_existed = file_path.exists();
                let _ = control_stream.write_all(b"150 Opening BINARY mode data connection\r\n").await;

                let mut transfer_success = false;
                let mut total_written: u64 = 0;
                let is_ascii = state.transfer_mode == "ascii";
                let rate_limiter = speed_limit_kbps.map(crate::core::rate_limiter::RateLimiter::new);

                if let Ok(mut data_stream) = transfer::get_data_connection(
                    state.passive_mode,
                    state.data_port,
                    &state.data_addr,
                    client_ip,
                    &mut state.passive_manager,
                ).await {
                    let abort = Arc::clone(&state.abort_flag);
                    let result = transfer::receive_file_with_limits(
                        &mut data_stream,
                        &file_path,
                        state.rest_offset,
                        abort,
                        is_ascii,
                        rate_limiter.as_ref(),
                    ).await;
                    match result {
                        Ok(written) => {
                            transfer_success = true;
                            total_written = written;
                        }
                        Err(e) => {
                            log::error!("STOR transfer error: {}", e);
                        }
                    }
                } else {
                    log::error!("STOR failed to get data connection for file: {}", file_path.display());
                }

                if state.passive_mode
                    && let Some(port) = state.data_port {
                        state.passive_manager.remove_listener(port);
                    }

                if transfer_success {
                    let _ = control_stream.write_all(b"226 Transfer complete\r\n").await;

                    let uploaded_size = tokio::fs::metadata(&file_path).await.map(|m| m.len()).unwrap_or(total_written);
                    
                    if quota_mb.is_some()
                        && let Err(e) = quota_manager.add_usage(state.current_user.as_deref().unwrap_or("anonymous"), uploaded_size).await {
                            log::error!("Failed to update quota usage: {}", e);
                    }
                    
                    if let Ok(mut fl) = file_logger.lock() {
                        if file_existed {
                            fl.log_update(
                                state.current_user.as_deref().unwrap_or("anonymous"),
                                client_ip,
                                &file_path.to_string_lossy(),
                                uploaded_size,
                                "FTP",
                            );
                        } else {
                            fl.log_upload(
                                state.current_user.as_deref().unwrap_or("anonymous"),
                                client_ip,
                                &file_path.to_string_lossy(),
                                uploaded_size,
                                "FTP",
                            );
                        }
                    }

                    if let Ok(mut log) = logger.lock() {
                        log.client_action(
                            "FTP",
                            &format!("Uploaded: {} ({} bytes) at offset {}", filename, uploaded_size, state.rest_offset),
                            client_ip,
                            state.current_user.as_deref(),
                            "UPLOAD",
                        );
                    }
                } else {
                    let _ = control_stream.write_all(b"451 Transfer failed\r\n").await;
                    if let Ok(mut fl) = file_logger.lock() {
                        fl.log_failed(
                            state.current_user.as_deref().unwrap_or("anonymous"),
                            client_ip,
                            "UPLOAD",
                            &file_path.to_string_lossy(),
                            "FTP",
                            "Transfer failed",
                        );
                    }
                }

                state.rest_offset = 0;
            } else {
                let _ = control_stream.write_all(b"501 Syntax error: STOR requires filename\r\n").await;
            }
        }

        APPE(filename) => {
            if !state.authenticated {
                let _ = control_stream.write_all(b"530 Not logged in\r\n").await;
                return Ok(true);
            }

            if let Some(filename) = filename {
                let can_append = {
                    let users = user_manager.lock()
                        .map_err(|_| anyhow::anyhow!("Failed to lock user manager"))?;
                    let user = state.current_user.as_ref().and_then(|u| users.get_user(u));
                    user.is_some_and(|u| u.permissions.can_append)
                };

                if !can_append {
                    let _ = control_stream.write_all(b"550 Permission denied\r\n").await;
                    return Ok(true);
                }

                let file_path = match state.resolve_path(filename) {
                    Ok(p) => p,
                    Err(e) => {
                        log::warn!("APPE failed for '{}': {}", filename, e);
                        let _ = control_stream.write_all(format!("550 {}\r\n", e).as_bytes()).await;
                        return Ok(true);
                    }
                };
                if !path_starts_with_ignore_case(&file_path, &state.home_dir) {
                    let _ = control_stream.write_all(b"550 Permission denied\r\n").await;
                    return Ok(true);
                }
                let _ = control_stream.write_all(b"150 Opening BINARY mode data connection for append\r\n").await;

                let is_ascii = state.transfer_mode == "ascii";

                if let Ok(mut data_stream) = transfer::get_data_connection(
                    state.passive_mode,
                    state.data_port,
                    &state.data_addr,
                    client_ip,
                    &mut state.passive_manager,
                ).await {
                    let abort = Arc::clone(&state.abort_flag);
                    let _ = transfer::receive_file_append(&mut data_stream, &file_path, abort, is_ascii).await;
                }

                if state.passive_mode
                    && let Some(port) = state.data_port {
                        state.passive_manager.remove_listener(port);
                    }

                let _ = control_stream.write_all(b"226 Transfer complete\r\n").await;

                let appended_size = tokio::fs::metadata(&file_path).await.map(|m| m.len()).unwrap_or(0);
                if let Ok(mut fl) = file_logger.lock() {
                    fl.log(crate::core::file_logger::FileLogInfo {
                        username: state.current_user.as_deref().unwrap_or("anonymous"),
                        client_ip,
                        operation: "APPEND",
                        file_path: &file_path.to_string_lossy(),
                        file_size: appended_size,
                        protocol: "FTP",
                        success: true,
                        message: "文件追加成功",
                    });
                }

                if let Ok(mut log) = logger.lock() {
                    log.client_action(
                        "FTP",
                        &format!("Appended: {}", filename),
                        client_ip,
                        state.current_user.as_deref(),
                        "APPEND",
                    );
                }
            }
        }

        DELE(filename) => {
            if !state.authenticated {
                let _ = control_stream.write_all(b"530 Not logged in\r\n").await;
                return Ok(true);
            }

            let can_delete = {
                let users = user_manager.lock()
                    .map_err(|_| anyhow::anyhow!("Failed to lock user manager"))?;
                let user = state.current_user.as_ref().and_then(|u| users.get_user(u));
                user.is_some_and(|u| u.permissions.can_delete)
            };

            if !can_delete {
                let _ = control_stream.write_all(b"550 Permission denied\r\n").await;
                return Ok(true);
            }

            if let Some(filename) = filename {
                let file_path = match state.resolve_path(filename) {
                    Ok(p) => p,
                    Err(e) => {
                        log::warn!("DELE failed for '{}': {}", filename, e);
                        let _ = control_stream.write_all(format!("550 {}\r\n", e).as_bytes()).await;
                        return Ok(true);
                    }
                };
                if !path_starts_with_ignore_case(&file_path, &state.home_dir) {
                    let _ = control_stream.write_all(b"550 Permission denied\r\n").await;
                    return Ok(true);
                }
                
                if !file_path.exists() {
                    let _ = control_stream.write_all(b"450 File unavailable: file not found\r\n").await;
                    return Ok(true);
                }
                
                if tokio::fs::remove_file(&file_path).await.is_ok() {
                    let _ = control_stream.write_all(b"250 File deleted\r\n").await;
                    if let Ok(mut fl) = file_logger.lock() {
                        fl.log_delete(
                            state.current_user.as_deref().unwrap_or("anonymous"),
                            client_ip,
                            &file_path.to_string_lossy(),
                            "FTP",
                        );
                    }
                    if let Ok(mut log) = logger.lock() {
                        log.client_action(
                            "FTP",
                            &format!("Deleted: {}", filename),
                            client_ip,
                            state.current_user.as_deref(),
                            "DELETE",
                        );
                    }
                } else {
                    let _ = control_stream.write_all(b"450 File unavailable: delete operation failed\r\n").await;
                }
            }
        }

        MKD(dirname) => {
            if !state.authenticated {
                let _ = control_stream.write_all(b"530 Not logged in\r\n").await;
                return Ok(true);
            }

            let can_mkdir = {
                let users = user_manager.lock()
                    .map_err(|_| anyhow::anyhow!("Failed to lock user manager"))?;
                let user = state.current_user.as_ref().and_then(|u| users.get_user(u));
                user.is_some_and(|u| u.permissions.can_mkdir)
            };

            if !can_mkdir {
                let _ = control_stream.write_all(b"550 Permission denied\r\n").await;
                return Ok(true);
            }

            if let Some(dirname) = dirname {
                let dir_path = match state.resolve_path(dirname) {
                    Ok(p) => p,
                    Err(e) => {
                        log::warn!("MKD failed for '{}': {}", dirname, e);
                        let _ = control_stream.write_all(format!("550 {}\r\n", e).as_bytes()).await;
                        return Ok(true);
                    }
                };
                if !path_starts_with_ignore_case(&dir_path, &state.home_dir) {
                    let _ = control_stream.write_all(b"550 Permission denied\r\n").await;
                    return Ok(true);
                }
                if tokio::fs::create_dir_all(&dir_path).await.is_ok() {
                    match to_ftp_path(&dir_path, std::path::Path::new(&state.home_dir)) {
                        Ok(ftp_path) => {
                            let _ = control_stream.write_all(format!("257 \"{}\" created\r\n", ftp_path).as_bytes()).await;
                        }
                        Err(e) => {
                            log::error!("MKD failed to get ftp path: {}", e);
                            let _ = control_stream.write_all(b"257 Directory created\r\n").await;
                        }
                    }
                    if let Ok(mut fl) = file_logger.lock() {
                        fl.log_mkdir(
                            state.current_user.as_deref().unwrap_or("anonymous"),
                            client_ip,
                            &dir_path.to_string_lossy(),
                            "FTP",
                        );
                    }
                    if let Ok(mut log) = logger.lock() {
                        log.client_action(
                            "FTP",
                            &format!("Created directory: {}", dirname),
                            client_ip,
                            state.current_user.as_deref(),
                            "MKDIR",
                        );
                    }
                } else {
                    let _ = control_stream.write_all(b"550 Create directory operation failed\r\n").await;
                }
            }
        }

        RMD(dirname) => {
            if !state.authenticated {
                let _ = control_stream.write_all(b"530 Not logged in\r\n").await;
                return Ok(true);
            }

            let can_rmdir = {
                let users = user_manager.lock()
                    .map_err(|_| anyhow::anyhow!("Failed to lock user manager"))?;
                let user = state.current_user.as_ref().and_then(|u| users.get_user(u));
                user.is_some_and(|u| u.permissions.can_rmdir)
            };

            if !can_rmdir {
                let _ = control_stream.write_all(b"550 Permission denied\r\n").await;
                return Ok(true);
            }

            if let Some(dirname) = dirname {
                let dir_path = match state.resolve_path(dirname) {
                    Ok(p) => p,
                    Err(e) => {
                        log::warn!("RMD failed for '{}': {}", dirname, e);
                        let _ = control_stream.write_all(format!("550 {}\r\n", e).as_bytes()).await;
                        return Ok(true);
                    }
                };
                if !path_starts_with_ignore_case(&dir_path, &state.home_dir) {
                    let _ = control_stream.write_all(b"550 Permission denied\r\n").await;
                    return Ok(true);
                }
                if tokio::fs::remove_dir_all(&dir_path).await.is_ok() {
                    let _ = control_stream.write_all(b"250 Directory removed\r\n").await;
                    if let Ok(mut fl) = file_logger.lock() {
                        fl.log_rmdir(
                            state.current_user.as_deref().unwrap_or("anonymous"),
                            client_ip,
                            &dir_path.to_string_lossy(),
                            "FTP",
                        );
                    }
                    if let Ok(mut log) = logger.lock() {
                        log.client_action(
                            "FTP",
                            &format!("Removed directory: {}", dirname),
                            client_ip,
                            state.current_user.as_deref(),
                            "RMDIR",
                        );
                    }
                } else {
                    let _ = control_stream.write_all(b"550 Remove directory operation failed\r\n").await;
                }
            }
        }

        RNFR(from_name) => {
            if !state.authenticated {
                let _ = control_stream.write_all(b"530 Not logged in\r\n").await;
                return Ok(true);
            }

            let can_rename = {
                let users = user_manager.lock()
                    .map_err(|_| anyhow::anyhow!("Failed to lock user manager"))?;
                let user = state.current_user.as_ref().and_then(|u| users.get_user(u));
                user.is_some_and(|u| u.permissions.can_rename)
            };

            if !can_rename {
                let _ = control_stream.write_all(b"550 Permission denied\r\n").await;
                return Ok(true);
            }

            if let Some(from_name) = from_name {
                let from_path = match state.resolve_path(from_name) {
                    Ok(p) => p,
                    Err(e) => {
                        log::warn!("RNFR failed for '{}': {}", from_name, e);
                        let _ = control_stream.write_all(format!("550 {}\r\n", e).as_bytes()).await;
                        return Ok(true);
                    }
                };
                log::info!("RNFR: raw='{}', resolved='{}', exists={}, starts_with={}", 
                    from_name, from_path.display(), from_path.exists(), path_starts_with_ignore_case(&from_path, &state.home_dir));
                if from_path.exists() && path_starts_with_ignore_case(&from_path, &state.home_dir) {
                    state.rename_from = Some(from_path.to_string_lossy().to_string());
                    let _ = control_stream.write_all(b"350 File exists, ready for destination name\r\n").await;
                    if let Ok(mut log) = logger.lock() {
                        log.client_action("FTP", &format!("RNFR: {}", from_path.display()), client_ip, state.current_user.as_deref(), "RNFR");
                    }
                } else {
                    log::warn!("RNFR failed: file not found or outside home - raw='{}', resolved='{}'", from_name, from_path.display());
                    let _ = control_stream.write_all(b"450 File unavailable: file not found\r\n").await;
                }
            } else {
                let _ = control_stream.write_all(b"501 Syntax error: RNFR requires filename\r\n").await;
            }
        }

        RNTO(to_name) => {
            if let Some(ref from_path) = state.rename_from {
                if let Some(to_name) = to_name {
                    let to_path = match state.resolve_path(to_name) {
                        Ok(p) => p,
                        Err(e) => {
                            log::warn!("RNTO failed for '{}': {}", to_name, e);
                            let _ = control_stream.write_all(format!("550 {}\r\n", e).as_bytes()).await;
                            state.rename_from = None;
                            return Ok(true);
                        }
                    };
                    log::info!("RNTO: raw='{}', resolved='{}', from='{}'", to_name, to_path.display(), from_path);
                    if !path_starts_with_ignore_case(&to_path, &state.home_dir) {
                        log::warn!("RNTO failed: destination outside home - {}", to_path.display());
                        let _ = control_stream.write_all(b"550 Permission denied\r\n").await;
                        state.rename_from = None;
                        return Ok(true);
                    }
                    let from_path_buf = PathBuf::from(from_path);
                    match tokio::fs::rename(&from_path_buf, &to_path).await {
                        Ok(()) => {
                            let _ = control_stream.write_all(b"250 Rename successful\r\n").await;
                            if let Ok(mut fl) = file_logger.lock() {
                                fl.log_rename(
                                    state.current_user.as_deref().unwrap_or("anonymous"),
                                    client_ip,
                                    from_path,
                                    &to_path.to_string_lossy(),
                                    "FTP",
                                );
                            }
                            if let Ok(mut log) = logger.lock() {
                                log.client_action(
                                    "FTP",
                                    &format!("Renamed: {} -> {}", from_path, to_path.display()),
                                    client_ip,
                                    state.current_user.as_deref(),
                                    "RENAME",
                                );
                            }
                        }
                        Err(e) => {
                            log::error!("Rename failed: {} -> {}: {} (os error {})", from_path, to_path.display(), e, e.raw_os_error().unwrap_or(0));
                            let _ = control_stream.write_all(b"550 Rename failed\r\n").await;
                        }
                    }
                } else {
                    let _ = control_stream.write_all(b"501 Syntax error: RNTO requires filename\r\n").await;
                }
            } else {
                let _ = control_stream.write_all(b"503 Bad sequence of commands\r\n").await;
            }
            state.rename_from = None;
        }

        SIZE(filename) => {
            if !state.authenticated {
                let _ = control_stream.write_all(b"530 Not logged in\r\n").await;
                return Ok(true);
            }
            if let Some(filename) = filename {
                let file_path = match state.resolve_path(filename) {
                    Ok(p) => p,
                    Err(e) => {
                        log::warn!("SIZE failed for '{}': {}", filename, e);
                        let _ = control_stream.write_all(format!("550 {}\r\n", e).as_bytes()).await;
                        return Ok(true);
                    }
                };
                if path_starts_with_ignore_case(&file_path, &state.home_dir) {
                    if let Ok(metadata) = tokio::fs::metadata(&file_path).await {
                        let _ = control_stream.write_all(format!("213 {}\r\n", metadata.len()).as_bytes()).await;
                    } else {
                        let _ = control_stream.write_all(b"450 File unavailable: file not found\r\n").await;
                    }
                } else {
                    let _ = control_stream.write_all(b"550 Permission denied\r\n").await;
                }
            }
        }

        MDTM(filename) => {
            if !state.authenticated {
                let _ = control_stream.write_all(b"530 Not logged in\r\n").await;
                return Ok(true);
            }
            if let Some(filename) = filename {
                let file_path = match state.resolve_path(filename) {
                    Ok(p) => p,
                    Err(e) => {
                        log::warn!("MDTM failed for '{}': {}", filename, e);
                        let _ = control_stream.write_all(format!("550 {}\r\n", e).as_bytes()).await;
                        return Ok(true);
                    }
                };
                if path_starts_with_ignore_case(&file_path, &state.home_dir) {
                    if let Ok(metadata) = tokio::fs::metadata(&file_path).await {
                        let mtime = transfer::get_file_mtime_raw(&metadata);
                        let _ = control_stream.write_all(format!("213 {}\r\n", mtime).as_bytes()).await;
                    } else {
                        let _ = control_stream.write_all(b"450 File unavailable: file not found\r\n").await;
                    }
                } else {
                    let _ = control_stream.write_all(b"550 Permission denied\r\n").await;
                }
            }
        }

        STOU => {
            if !state.authenticated {
                let _ = control_stream.write_all(b"530 Not logged in\r\n").await;
                return Ok(true);
            }

            let can_write = {
                let users = user_manager.lock()
                    .map_err(|_| anyhow::anyhow!("Failed to lock user manager"))?;
                let user = state.current_user.as_ref().and_then(|u| users.get_user(u));
                user.is_some_and(|u| u.permissions.can_write)
            };

            if !can_write {
                let _ = control_stream.write_all(b"550 Permission denied\r\n").await;
                return Ok(true);
            }

            let unique_name = format!("stou_{}_{}", 
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
                rand::random::<u32>()
            );
            
            let file_path = match state.resolve_path(&unique_name) {
                Ok(p) => p,
                Err(e) => {
                    log::warn!("STOU failed: {}", e);
                    let _ = control_stream.write_all(format!("550 {}\r\n", e).as_bytes()).await;
                    return Ok(true);
                }
            };
            if !path_starts_with_ignore_case(&file_path, &state.home_dir) {
                let _ = control_stream.write_all(b"550 Permission denied\r\n").await;
                return Ok(true);
            }

            let _ = control_stream.write_all(format!("150 FILE: {}\r\n", unique_name).as_bytes()).await;

            let is_ascii = state.transfer_mode == "ascii";

            if let Ok(mut data_stream) = transfer::get_data_connection(
                state.passive_mode,
                state.data_port,
                &state.data_addr,
                client_ip,
                &mut state.passive_manager,
            ).await {
                let abort = Arc::clone(&state.abort_flag);
                let _ = transfer::receive_file(&mut data_stream, &file_path, 0, abort, is_ascii).await;
            }

            if state.passive_mode
                && let Some(port) = state.data_port {
                    state.passive_manager.remove_listener(port);
                }

            let _ = control_stream.write_all(b"226 Transfer complete\r\n").await;

            let uploaded_size = tokio::fs::metadata(&file_path).await.map(|m| m.len()).unwrap_or(0);
            if let Ok(mut fl) = file_logger.lock() {
                fl.log_upload(
                    state.current_user.as_deref().unwrap_or("anonymous"),
                    client_ip,
                    &file_path.to_string_lossy(),
                    uploaded_size,
                    "FTP",
                );
            }

            if let Ok(mut log) = logger.lock() {
                log.client_action(
                    "FTP",
                    &format!("Uploaded unique file: {}", unique_name),
                    client_ip,
                    state.current_user.as_deref(),
                    "UPLOAD",
                );
            }
        }

        Unknown(cmd_str) => {
            let _ = control_stream.write_all(format!("202 Command not implemented: {}\r\n", cmd_str).as_bytes()).await;
        }
    }

    Ok(true)
}
