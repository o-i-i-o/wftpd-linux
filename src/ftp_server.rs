use anyhow::Result;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, SocketAddr};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::users::UserManager;
use crate::logger::Logger;

pub struct FtpServer {
    config: Arc<Mutex<Config>>,
    user_manager: Arc<Mutex<UserManager>>,
    logger: Arc<Mutex<Logger>>,
    running: Arc<Mutex<bool>>,
    connections: Arc<Mutex<HashMap<String, ConnectionInfo>>>,
}

#[derive(Clone)]
struct ConnectionInfo {
    connected_at: Instant,
    remote_addr: SocketAddr,
    username: Option<String>,
}

impl FtpServer {
    pub fn new(config: Arc<Mutex<Config>>, user_manager: Arc<Mutex<UserManager>>, 
               logger: Arc<Mutex<Logger>>) -> Self {
        FtpServer {
            config,
            user_manager,
            logger,
            running: Arc::new(Mutex::new(false)),
            connections: Arc::new(Mutex::new(HashMap::new())),
        }
    }
    
    pub fn start(&self) -> Result<()> {
        let (bind_ip, ftp_port) = {
            let cfg = self.config.lock().unwrap();
            (cfg.server.bind_ip.clone(), cfg.server.ftp_port)
        };
        let bind_addr = format!("{}:{}", bind_ip, ftp_port);
        let listener = TcpListener::bind(&bind_addr)?;
        
        {
            let mut running = self.running.lock().unwrap();
            *running = true;
        }
        
        let config = Arc::clone(&self.config);
        let user_manager = Arc::clone(&self.user_manager);
        let logger = Arc::clone(&self.logger);
        let running = Arc::clone(&self.running);
        
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let is_running = *running.lock().unwrap();
                if !is_running {
                    break;
                }
                
                match stream {
                    Ok(stream) => {
                        let config = Arc::clone(&config);
                        let user_manager = Arc::clone(&user_manager);
                        let logger = Arc::clone(&logger);
                        
                        std::thread::spawn(move || {
                            if let Err(_e) = handle_ftp_connection(stream, &config, &user_manager, &logger) {
                            }
                        });
                    }
                    Err(e) => {
                        eprintln!("Failed to accept connection: {}", e);
                    }
                }
            }
        });
        
        Ok(())
    }
    
    pub fn stop(&self) {
        let mut running = self.running.lock().unwrap();
        *running = false;
    }
    
    pub fn is_running(&self) -> bool {
        *self.running.lock().unwrap()
    }
}

fn handle_ftp_connection(mut stream: TcpStream, 
                         config: &Arc<Mutex<Config>>,
                         user_manager: &Arc<Mutex<UserManager>>,
                         logger: &Arc<Mutex<Logger>>) -> Result<()> {
    let remote_addr = stream.peer_addr()?;
    let remote_ip = remote_addr.ip().to_string();
    
    {
        let cfg = config.lock().unwrap();
        if !cfg.is_ip_allowed(&remote_ip) {
            let response = b"530 Connection denied by IP filter\r\n";
            stream.write_all(response)?;
            return Ok(());
        }
    }
    
    let welcome_msg;
    {
        let cfg = config.lock().unwrap();
        welcome_msg = cfg.ftp.welcome_message.clone();
    }
    stream.write_all(format!("220 {} \r\n", welcome_msg).as_bytes())?;
    
    let mut current_user: Option<String> = None;
    let mut authenticated = false;
    let mut data_port: Option<u16> = None;
    let mut passive_mode = false;
    let mut cwd;
    {
        let cfg = config.lock().unwrap();
        cwd = cfg.ftp.default_home.clone();
    }
    
    let mut buffer = [0u8; 4096];
    
    loop {
        let conn_timeout;
        {
            let cfg = config.lock().unwrap();
            conn_timeout = cfg.server.connection_timeout;
        }
        stream.set_read_timeout(Some(Duration::from_secs(conn_timeout)))?;
        let bytes_read = stream.read(&mut buffer)?;
        
        if bytes_read == 0 {
            break;
        }
        
        let command = String::from_utf8_lossy(&buffer[..bytes_read]).trim().to_string();
        
        let parts: Vec<&str> = command.splitn(2, ' ').collect();
        let cmd = parts[0].to_uppercase();
        let arg = parts.get(1).map(|s| s.trim());
        
        match cmd.as_str() {
            "USER" => {
                if let Some(username) = arg {
                    current_user = Some(username.to_string());
                    stream.write_all(b"331 User name okay, need password\r\n")?;
                } else {
                    stream.write_all(b"501 Syntax error in parameters or arguments\r\n")?;
                }
            }
            
            "PASS" => {
                if let Some(ref username) = current_user {
                    let password = arg.unwrap_or("");
                    let mut users = user_manager.lock().unwrap();
                    match users.authenticate(username, password) {
                        Ok(true) => {
                            authenticated = true;
                            if let Some(user) = users.get_user(username) {
                                cwd = user.home_dir.clone();
                            }
                            stream.write_all(b"230 User logged in\r\n")?;
                            logger.lock().unwrap().client_action("FTP", 
                                &format!("User {} logged in", username), 
                                &remote_ip, Some(username), "LOGIN");
                        }
                        Ok(false) => {
                            stream.write_all(b"530 Not logged in, user cannot be authenticated\r\n")?;
                        }
                        Err(_) => {
                            stream.write_all(b"530 Not logged in\r\n")?;
                        }
                    }
                } else {
                    stream.write_all(b"530 Please login with USER and PASS\r\n")?;
                }
            }
            
            "QUIT" => {
                stream.write_all(b"221 Goodbye\r\n")?;
                break;
            }
            
            "SYST" => {
                stream.write_all(b"215 UNIX Type: L8\r\n")?;
            }
            
            "FEAT" => {
                stream.write_all(b"211-Features:\r\n SIZE\r\n MDTM\r\n211 End\r\n")?;
            }
            
            "OPTS" => {
                stream.write_all(b"200 Options set\r\n")?;
            }
            
            "PWD" | "XPWD" => {
                stream.write_all(format!("257 \"{}\"\r\n", cwd).as_bytes())?;
            }
            
            "CWD" => {
                if !authenticated {
                    stream.write_all(b"530 Not logged in\r\n")?;
                    continue;
                }
                
                if let Some(dir) = arg {
                    let new_path = if dir.starts_with('/') {
                        Path::new(dir).to_path_buf()
                    } else {
                        Path::new(&cwd).join(dir)
                    };
                    
                    if new_path.exists() && new_path.is_dir() {
                        cwd = new_path.to_string_lossy().to_string();
                        stream.write_all(format!("250 \"{}\" is current directory\r\n", cwd).as_bytes())?;
                    } else {
                        stream.write_all(b"550 Failed to change directory\r\n")?;
                    }
                }
            }
            
            "CDUP" | "XCUP" => {
                if let Some(parent) = Path::new(&cwd).parent() {
                    cwd = parent.to_string_lossy().to_string();
                    stream.write_all(b"250 Directory changed\r\n")?;
                }
            }
            
            "TYPE" => {
                stream.write_all(b"200 Type set to I\r\n")?;
            }
            
            "PASV" => {
                passive_mode = true;
                let (port_min, _port_max) = {
                    let cfg = config.lock().unwrap();
                    cfg.ftp.passive_ports
                };
                let port = port_min;
                data_port = Some(port);
                let ip_octets = remote_ip.replace('.', ",");
                stream.write_all(format!("227 Entering Passive Mode ({},{})\r\n", ip_octets, port).as_bytes())?;
            }
            
            "PORT" => {
                if let Some(data) = arg {
                    let parts: Vec<u16> = data.split(',')
                        .filter_map(|s| s.parse().ok())
                        .collect();
                    if parts.len() == 6 {
                        data_port = Some(parts[4] * 256 + parts[5]);
                        passive_mode = false;
                        stream.write_all(b"200 PORT command successful\r\n")?;
                    } else {
                        stream.write_all(b"501 Syntax error in parameters or arguments\r\n")?;
                    }
                }
            }
            
            "LIST" | "NLST" => {
                if !authenticated {
                    stream.write_all(b"530 Not logged in\r\n")?;
                    continue;
                }
                
                stream.write_all(b"150 Here comes the directory listing\r\n")?;
                
                if let Some(port) = data_port {
                    if let Ok(mut data_stream) = TcpStream::connect(format!("{}:{}", 
                        if passive_mode { "127.0.0.1" } else { &remote_ip }, port)) {
                        
                        let path = Path::new(&cwd);
                        if let Ok(entries) = std::fs::read_dir(path) {
                            for entry in entries.flatten() {
                                if let Ok(_metadata) = entry.metadata() {
                                    let name = entry.file_name().to_string_lossy().to_string();
                                    let line = format!("{}\r\n", name);
                                    let _ = data_stream.write_all(line.as_bytes());
                                }
                            }
                        }
                    }
                }
                
                stream.write_all(b"226 Transfer complete\r\n")?;
            }
            
            "RETR" => {
                if !authenticated {
                    stream.write_all(b"530 Not logged in\r\n")?;
                    continue;
                }
                
                if let Some(filename) = arg {
                    let file_path = Path::new(&cwd).join(filename);
                    
                    if !file_path.exists() || !file_path.is_file() {
                        stream.write_all(b"550 File not found\r\n")?;
                        continue;
                    }
                    
                    {
                        let users = user_manager.lock().unwrap();
                        let user = current_user.as_ref().and_then(|u| users.get_user(u));
                        
                        if let Some(user) = user {
                            if !user.permissions.can_read {
                                stream.write_all(b"550 Permission denied\r\n")?;
                                continue;
                            }
                        }
                    }
                    
                    stream.write_all(b"150 Opening BINARY mode data connection\r\n")?;
                    
                    if let Ok(mut file) = std::fs::File::open(&file_path) {
                        if let Ok(mut data_stream) = TcpStream::connect(format!("{}:{}", 
                            if passive_mode { "127.0.0.1" } else { &remote_ip }, data_port.unwrap_or(0))) {
                            let mut buf = [0u8; 8192];
                            loop {
                                match file.read(&mut buf) {
                                    Ok(0) => break,
                                    Ok(n) => {
                                        if data_stream.write_all(&buf[..n]).is_err() {
                                            break;
                                        }
                                    }
                                    Err(_) => break,
                                }
                            }
                        }
                    }
                    
                    stream.write_all(b"226 Transfer complete\r\n")?;
                    
                    logger.lock().unwrap().client_action("FTP", 
                        &format!("Downloaded: {}", filename),
                        &remote_ip, 
                        current_user.as_deref(), "DOWNLOAD");
                }
            }
            
            "STOR" | "APPE" => {
                if !authenticated {
                    stream.write_all(b"530 Not logged in\r\n")?;
                    continue;
                }
                
                if let Some(filename) = arg {
                    {
                        let users = user_manager.lock().unwrap();
                        let user = current_user.as_ref().and_then(|u| users.get_user(u));
                        
                        if let Some(user) = user {
                            if !user.permissions.can_write {
                                stream.write_all(b"550 Permission denied\r\n")?;
                                continue;
                            }
                        }
                    }
                    
                    let file_path = Path::new(&cwd).join(filename);
                    stream.write_all(b"150 Opening BINARY mode data connection\r\n")?;
                    
                    if let Ok(mut data_stream) = TcpStream::connect(format!("{}:{}", 
                        if passive_mode { "127.0.0.1" } else { &remote_ip }, data_port.unwrap_or(0))) {
                        if let Ok(mut file) = std::fs::File::create(&file_path) {
                            let mut buf = [0u8; 8192];
                            loop {
                                match data_stream.read(&mut buf) {
                                    Ok(0) => break,
                                    Ok(n) => {
                                        if file.write_all(&buf[..n]).is_err() {
                                            break;
                                        }
                                    }
                                    Err(_) => break,
                                }
                            }
                        }
                    }
                    
                    stream.write_all(b"226 Transfer complete\r\n")?;
                    
                    logger.lock().unwrap().client_action("FTP", 
                        &format!("Uploaded: {}", filename),
                        &remote_ip, 
                        current_user.as_deref(), "UPLOAD");
                }
            }
            
            "DELE" => {
                if !authenticated {
                    stream.write_all(b"530 Not logged in\r\n")?;
                    continue;
                }
                
                {
                    let users = user_manager.lock().unwrap();
                    let user = current_user.as_ref().and_then(|u| users.get_user(u));
                    
                    if let Some(user) = user {
                        if !user.permissions.can_delete {
                            stream.write_all(b"550 Permission denied\r\n")?;
                            continue;
                        }
                    }
                }
                
                if let Some(filename) = arg {
                    let file_path = Path::new(&cwd).join(filename);
                    if std::fs::remove_file(&file_path).is_ok() {
                        stream.write_all(b"250 File deleted\r\n")?;
                        logger.lock().unwrap().client_action("FTP", 
                            &format!("Deleted: {}", filename),
                            &remote_ip, 
                            current_user.as_deref(), "DELETE");
                    } else {
                        stream.write_all(b"550 Delete operation failed\r\n")?;
                    }
                }
            }
            
            "MKD" | "XMKD" => {
                if !authenticated {
                    stream.write_all(b"530 Not logged in\r\n")?;
                    continue;
                }
                
                {
                    let users = user_manager.lock().unwrap();
                    let user = current_user.as_ref().and_then(|u| users.get_user(u));
                    
                    if let Some(user) = user {
                        if !user.permissions.can_mkdir {
                            stream.write_all(b"550 Permission denied\r\n")?;
                            continue;
                        }
                    }
                }
                
                if let Some(dirname) = arg {
                    let dir_path = Path::new(&cwd).join(dirname);
                    if std::fs::create_dir_all(&dir_path).is_ok() {
                        stream.write_all(format!("257 \"{}\" created\r\n", dir_path.display()).as_bytes())?;
                        logger.lock().unwrap().client_action("FTP", 
                            &format!("Created directory: {}", dirname),
                            &remote_ip, 
                            current_user.as_deref(), "MKDIR");
                    } else {
                        stream.write_all(b"550 Create directory operation failed\r\n")?;
                    }
                }
            }
            
            "RMD" | "XRMD" => {
                if !authenticated {
                    stream.write_all(b"530 Not logged in\r\n")?;
                    continue;
                }
                
                {
                    let users = user_manager.lock().unwrap();
                    let user = current_user.as_ref().and_then(|u| users.get_user(u));
                    
                    if let Some(user) = user {
                        if !user.permissions.can_rmdir {
                            stream.write_all(b"550 Permission denied\r\n")?;
                            continue;
                        }
                    }
                }
                
                if let Some(dirname) = arg {
                    let dir_path = Path::new(&cwd).join(dirname);
                    if std::fs::remove_dir_all(&dir_path).is_ok() {
                        stream.write_all(b"250 Directory removed\r\n")?;
                        logger.lock().unwrap().client_action("FTP", 
                            &format!("Removed directory: {}", dirname),
                            &remote_ip, 
                            current_user.as_deref(), "RMDIR");
                    } else {
                        stream.write_all(b"550 Remove directory operation failed\r\n")?;
                    }
                }
            }
            
            "RNFR" | "RNTO" => {
                if !authenticated {
                    stream.write_all(b"530 Not logged in\r\n")?;
                    continue;
                }
                
                {
                    let users = user_manager.lock().unwrap();
                    let user = current_user.as_ref().and_then(|u| users.get_user(u));
                    
                    if let Some(user) = user {
                        if !user.permissions.can_rename {
                            stream.write_all(b"550 Permission denied\r\n")?;
                            continue;
                        }
                    }
                }
                
                stream.write_all(b"350 File exists, ready for destination name\r\n")?;
            }
            
            "SIZE" => {
                if let Some(filename) = arg {
                    let file_path = Path::new(&cwd).join(filename);
                    if let Ok(metadata) = std::fs::metadata(&file_path) {
                        stream.write_all(format!("213 {}\r\n", metadata.len()).as_bytes())?;
                    } else {
                        stream.write_all(b"550 File not found\r\n")?;
                    }
                }
            }
            
            "MDTM" => {
                stream.write_all(b"213 0\r\n")?;
            }
            
            "NOOP" => {
                stream.write_all(b"200 OK\r\n")?;
            }
            
            "STAT" => {
                stream.write_all(b"211 FTP server status\r\n")?;
            }
            
            _ => {
                stream.write_all(b"202 Command not implemented\r\n")?;
            }
        }
    }
    
    Ok(())
}
