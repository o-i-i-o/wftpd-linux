use anyhow::Result;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use crate::core::config::Config;
use crate::core::file_logger::FileLogger;
use crate::core::logger::Logger;
use crate::core::users::UserManager;

use super::data_connection::PassiveListenerMap;
use super::rate_limit::RateLimiter;

pub struct FtpSession {
    pub stream: TcpStream,
    pub config: Arc<std::sync::Mutex<Config>>,
    pub user_manager: Arc<std::sync::Mutex<UserManager>>,
    pub logger: Arc<std::sync::Mutex<Logger>>,
    pub file_logger: Arc<std::sync::Mutex<FileLogger>>,
    pub passive_listeners: PassiveListenerMap,
    pub rate_limiter: Arc<RateLimiter>,
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
}

impl FtpSession {
    pub fn new(
        stream: TcpStream,
        config: Arc<std::sync::Mutex<Config>>,
        user_manager: Arc<std::sync::Mutex<UserManager>>,
        logger: Arc<std::sync::Mutex<Logger>>,
        file_logger: Arc<std::sync::Mutex<FileLogger>>,
        passive_listeners: PassiveListenerMap,
        rate_limiter: Arc<RateLimiter>,
    ) -> Result<Self> {
        let remote_addr = stream.peer_addr()?;
        let remote_ip = remote_addr.ip().to_string();
        
        let local_ip = stream.local_addr()
            .ok()
            .map(|addr| addr.ip().to_string());

        let (cwd, home_dir) = {
            let cfg = config.lock().unwrap();
            let default_home = cfg.ftp.default_home.clone();
            let home_path = Path::new(&default_home);
            
            if !home_path.exists() {
                logger.lock().unwrap().warning(
                    "FTP",
                    &format!("Default home directory does not exist: {}", default_home),
                );
            }
            
            (default_home.clone(), default_home)
        };

        Ok(Self {
            stream,
            config,
            user_manager,
            logger,
            file_logger,
            passive_listeners,
            rate_limiter,
            remote_ip,
            local_ip,
            current_user: None,
            authenticated: false,
            cwd,
            home_dir,
            data_port: None,
            data_addr: None,
            passive_mode: false,
            rest_offset: 0,
            rename_from: None,
            abort_flag: Arc::new(AtomicBool::new(false)),
            utf8_enabled: true,
            binary_transfer: true,
        })
    }

    pub fn run(&mut self) -> Result<()> {
        if let Err(_e) = self.rate_limiter.check_and_record(&self.remote_ip) {
            self.logger.lock().unwrap().warning(
                "FTP",
                &format!("Rate limit exceeded for {}", self.remote_ip),
            );
            let _ = self.stream.write_all(b"421 Too many connections, try again later\r\n");
            return Ok(());
        }

        self.logger.lock().unwrap().client_action(
            "FTP",
            &format!("Client connected from {}", self.remote_ip),
            &self.remote_ip,
            None,
            "CONNECT",
        );

        {
            let cfg = self.config.lock().unwrap();
            if !cfg.is_ip_allowed(&self.remote_ip) {
                if let Ok(mut log) = self.logger.try_lock() {
                    log.warning("FTP", &format!("Connection rejected from {} by IP filter", self.remote_ip));
                }
                let _ = self.stream.write_all(b"530 Connection denied by IP filter\r\n");
                self.rate_limiter.release_for_ip(&self.remote_ip);
                return Ok(());
            }
        }

        let welcome_msg = {
            let cfg = self.config.lock().unwrap();
            cfg.ftp.welcome_message.clone()
        };
        self.stream.write_all(format!("220 {} \r\n", welcome_msg).as_bytes())?;

        let mut buffer = [0u8; 4096];

        loop {
            let conn_timeout = {
                let cfg = self.config.lock().unwrap();
                cfg.server.connection_timeout
            };
            self.stream.set_read_timeout(Some(Duration::from_secs(conn_timeout)))?;
            let bytes_read = self.stream.read(&mut buffer)?;

            if bytes_read == 0 {
                break;
            }

            let command = String::from_utf8_lossy(&buffer[..bytes_read])
                .trim()
                .to_string();

            let parts: Vec<&str> = command.splitn(2, ' ').collect();
            let cmd = parts[0].to_uppercase();
            let arg = parts.get(1).map(|s| s.trim());

            if self.handle_command(&cmd, arg)? {
                break;
            }
        }

        self.rate_limiter.release_for_ip(&self.remote_ip);
        Ok(())
    }

    fn handle_command(&mut self, cmd: &str, arg: Option<&str>) -> Result<bool> {
        match cmd {
            "USER" => self.cmd_user(arg)?,
            "PASS" => self.cmd_pass(arg)?,
            "QUIT" => {
                self.stream.write_all(b"221 Goodbye\r\n")?;
                return Ok(true);
            }
            "SYST" => self.stream.write_all(b"215 UNIX Type: L8\r\n")?,
            "FEAT" => self.stream.write_all(b"211-Features:\r\n SIZE\r\n MDTM\r\n REST STREAM\r\n PASV\r\n EPSV\r\n EPRT\r\n PORT\r\n MLST\r\n MLSD\r\n MODE\r\n STRU\r\n UTF8\r\n TVFS\r\n211 End\r\n")?,
            "HELP" => self.cmd_help(arg)?,
            "MODE" => self.cmd_mode(arg)?,
            "STRU" => self.cmd_stru(arg)?,
            "ALLO" => self.stream.write_all(b"200 ALLO command successful\r\n")?,
            "OPTS" => self.cmd_opts(arg)?,
            "PWD" | "XPWD" => self.stream.write_all(format!("257 \"{}\"\r\n", self.cwd).as_bytes())?,
            "CWD" => self.cmd_cwd(arg)?,
            "CDUP" | "XCUP" => self.cmd_cdup()?,
            "TYPE" => self.cmd_type(arg)?,
            "MLST" => self.cmd_mlst(arg)?,
            "REST" => self.cmd_rest(arg)?,
            "PASV" => self.cmd_pasv()?,
            "EPSV" => self.cmd_epsv()?,
            "PORT" => self.cmd_port(arg)?,
            "EPRT" => self.cmd_eprt(arg)?,
            "PBSZ" => self.stream.write_all(b"200 PBSZ=0\r\n")?,
            "PROT" => self.cmd_prot(arg)?,
            "LIST" | "NLST" => self.cmd_list(arg)?,
            "MLSD" => self.cmd_mlsd()?,
            "RETR" => self.cmd_retr(arg)?,
            "STOR" => self.cmd_stor(arg)?,
            "APPE" => self.cmd_appe(arg)?,
            "DELE" => self.cmd_dele(arg)?,
            "MKD" | "XMKD" => self.cmd_mkd(arg)?,
            "RMD" | "XRMD" => self.cmd_rmd(arg)?,
            "RNFR" => self.cmd_rnfr(arg)?,
            "RNTO" => self.cmd_rnto(arg)?,
            "SIZE" => self.cmd_size(arg)?,
            "MDTM" => self.cmd_mdtm(arg)?,
            "NOOP" => self.stream.write_all(b"200 OK\r\n")?,
            "STAT" => self.cmd_stat()?,
            "ABOR" => self.handle_abor()?,
            _ => self.stream.write_all(b"202 Command not implemented\r\n")?,
        }
        Ok(false)
    }

    pub fn cleanup_data_connection(&mut self) {
        if self.passive_mode {
            if let Some(port) = self.data_port {
                let mut listeners = self.passive_listeners.lock().unwrap();
                listeners.remove(&port);
            }
        }
    }
}
