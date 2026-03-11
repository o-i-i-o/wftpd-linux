use anyhow::Result;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::config::Config;
use crate::users::UserManager;
use crate::logger::Logger;

pub struct FtpServer {
    config: Arc<Mutex<Config>>,
    user_manager: Arc<Mutex<UserManager>>,
    logger: Arc<Mutex<Logger>>,
    running: Arc<Mutex<bool>>,
    passive_listeners: Arc<Mutex<HashMap<u16, Arc<Mutex<Option<TcpListener>>>>>>,
}

impl FtpServer {
    pub fn new(config: Arc<Mutex<Config>>, user_manager: Arc<Mutex<UserManager>>, 
               logger: Arc<Mutex<Logger>>) -> Self {
        FtpServer {
            config,
            user_manager,
            logger,
            running: Arc::new(Mutex::new(false)),
            passive_listeners: Arc::new(Mutex::new(HashMap::new())),
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
        let passive_listeners = Arc::clone(&self.passive_listeners);
        
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
                        let passive_listeners = Arc::clone(&passive_listeners);
                        
                        std::thread::spawn(move || {
                            if let Err(_e) = handle_ftp_connection(stream, &config, &user_manager, &logger, &passive_listeners) {
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
        
        let mut listeners = self.passive_listeners.lock().unwrap();
        listeners.clear();
    }
    
    pub fn is_running(&self) -> bool {
        *self.running.lock().unwrap()
    }
}

fn handle_ftp_connection(mut stream: TcpStream, 
                         config: &Arc<Mutex<Config>>,
                         user_manager: &Arc<Mutex<UserManager>>,
                         logger: &Arc<Mutex<Logger>>,
                         passive_listeners: &Arc<Mutex<HashMap<u16, Arc<Mutex<Option<TcpListener>>>>>>) -> Result<()> {
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
    
    let mut rest_offset: u64 = 0;
    let mut _tls_enabled = false;
    let mut _tls_context: Option<TlsContext> = None;
    let mut _data_tls_enabled = false;
    
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
                stream.write_all(b"211-Features:\r\n SIZE\r\n MDTM\r\n REST STREAM\r\n AUTH TLS\r\n PBSZ\r\n PROT\r\n PASV\r\n EPSV\r\n MLST\r\n MLSD\r\n211 End\r\n")?;
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
            
            "REST" => {
                if let Some(offset_str) = arg {
                    if let Ok(offset) = offset_str.parse::<u64>() {
                        rest_offset = offset;
                        stream.write_all(format!("350 Restarting at {}\r\n", offset).as_bytes())?;
                        logger.lock().unwrap().client_action("FTP",
                            &format!("REST command: offset {}", offset),
                            &remote_ip,
                            current_user.as_deref(), "REST");
                    } else {
                        stream.write_all(b"501 Syntax error in REST parameter\r\n")?;
                    }
                } else {
                    rest_offset = 0;
                    stream.write_all(b"350 Restarting at 0\r\n")?;
                }
            }
            
            "PASV" => {
                passive_mode = true;
                let (port_min, port_max) = {
                    let cfg = config.lock().unwrap();
                    cfg.ftp.passive_ports
                };
                
                let passive_port = find_available_passive_port(passive_listeners, port_min, port_max)?;
                
                let bind_ip = config.lock().unwrap().server.bind_ip.clone();
                let passive_listener = TcpListener::bind(format!("{}:{}", bind_ip, passive_port))?;
                passive_listener.set_nonblocking(true)?;
                
                {
                    let mut listeners = passive_listeners.lock().unwrap();
                    listeners.insert(passive_port, Arc::new(Mutex::new(Some(passive_listener))));
                }
                
                data_port = Some(passive_port);
                
                let ip_octets = remote_ip.replace('.', ",");
                stream.write_all(format!("227 Entering Passive Mode ({},{},{})\r\n", 
                    ip_octets, passive_port >> 8, passive_port & 0xFF).as_bytes())?;
                
                logger.lock().unwrap().client_action("FTP",
                    &format!("PASV mode: port {}", passive_port),
                    &remote_ip,
                    current_user.as_deref(), "PASV");
            }
            
            "EPSV" => {
                passive_mode = true;
                let (port_min, port_max) = {
                    let cfg = config.lock().unwrap();
                    cfg.ftp.passive_ports
                };
                
                let passive_port = find_available_passive_port(passive_listeners, port_min, port_max)?;
                
                let bind_ip = config.lock().unwrap().server.bind_ip.clone();
                let passive_listener = TcpListener::bind(format!("{}:{}", bind_ip, passive_port))?;
                passive_listener.set_nonblocking(true)?;
                
                {
                    let mut listeners = passive_listeners.lock().unwrap();
                    listeners.insert(passive_port, Arc::new(Mutex::new(Some(passive_listener))));
                }
                
                data_port = Some(passive_port);
                stream.write_all(format!("229 Entering Extended Passive Mode (|||{}|)\r\n", passive_port).as_bytes())?;
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
            
            "AUTH" => {
                if let Some(method) = arg {
                    if method.to_uppercase() == "TLS" || method.to_uppercase() == "SSL" {
                        stream.write_all(b"234 AUTH TLS OK\r\n")?;
                        
                        let cert_path = config.lock().unwrap().security.cert_path.clone();
                        let key_path = config.lock().unwrap().security.key_path.clone();
                        
                        if let (Some(cert), Some(key)) = (cert_path, key_path) {
                            match create_tls_context(&cert, &key) {
                                Ok(_ctx) => {
                                    _tls_context = Some(_ctx);
                                    _tls_enabled = true;
                                    logger.lock().unwrap().client_action("FTP",
                                        "TLS/SSL enabled",
                                        &remote_ip,
                                        current_user.as_deref(), "AUTH");
                                }
                                Err(_) => {
                                    stream.write_all(b"421 TLS initialization failed\r\n")?;
                                }
                            }
                        } else {
                            stream.write_all(b"421 TLS not configured\r\n")?;
                        }
                    } else {
                        stream.write_all(b"504 Auth type not supported\r\n")?;
                    }
                }
            }
            
            "PBSZ" => {
                stream.write_all(b"200 PBSZ=0\r\n")?;
            }
            
            "PROT" => {
                if let Some(level) = arg {
                    match level.to_uppercase().as_str() {
                        "P" => {
                            _data_tls_enabled = true;
                            stream.write_all(b"200 PROT Private\r\n")?;
                        }
                        "C" => {
                            _data_tls_enabled = false;
                            stream.write_all(b"200 PROT Clear\r\n")?;
                        }
                        _ => {
                            stream.write_all(b"504 PROT level not supported\r\n")?;
                        }
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
                    let data_result = if passive_mode {
                        let listener_arc = {
                            let listeners = passive_listeners.lock().unwrap();
                            listeners.get(&port).cloned()
                        };
                        
                        if let Some(listener_arc) = listener_arc {
                            let mut listener_guard = listener_arc.lock().unwrap();
                            if let Some(listener) = listener_guard.take() {
                                listener.set_nonblocking(false)?;
                                listener.accept().map(|(s, _)| s)
                            } else {
                                Err(std::io::Error::new(std::io::ErrorKind::NotFound, "No passive listener"))
                            }
                        } else {
                            Err(std::io::Error::new(std::io::ErrorKind::NotFound, "No passive listener"))
                        }
                    } else {
                        TcpStream::connect(format!("{}:{}", &remote_ip, port))
                    };
                    
                    if let Ok(mut data_stream) = data_result {
                        let path = Path::new(&cwd);
                        if let Ok(entries) = std::fs::read_dir(path) {
                            for entry in entries.flatten() {
                                if let Ok(metadata) = entry.metadata() {
                                    let name = entry.file_name().to_string_lossy().to_string();
                                    let perms = if metadata.is_dir() { "drwxr-xr-x" } else { "-rw-r--r--" };
                                    let size = metadata.len();
                                    let mtime = get_file_mtime(&metadata);
                                    let line = format!("{} 1 user user {:>10} {} {}\r\n", 
                                        perms, size, mtime, name);
                                    let _ = data_stream.write_all(line.as_bytes());
                                }
                            }
                        }
                    }
                    
                    if passive_mode {
                        let mut listeners = passive_listeners.lock().unwrap();
                        listeners.remove(&port);
                    }
                }
                
                stream.write_all(b"226 Transfer complete\r\n")?;
            }
            
            "MLSD" => {
                if !authenticated {
                    stream.write_all(b"530 Not logged in\r\n")?;
                    continue;
                }
                
                stream.write_all(b"150 Here comes the directory listing\r\n")?;
                
                if let Some(port) = data_port {
                    let data_result = if passive_mode {
                        let listener_arc = {
                            let listeners = passive_listeners.lock().unwrap();
                            listeners.get(&port).cloned()
                        };
                        
                        if let Some(listener_arc) = listener_arc {
                            let mut listener_guard = listener_arc.lock().unwrap();
                            if let Some(listener) = listener_guard.take() {
                                listener.set_nonblocking(false)?;
                                listener.accept().map(|(s, _)| s)
                            } else {
                                Err(std::io::Error::new(std::io::ErrorKind::NotFound, "No passive listener"))
                            }
                        } else {
                            Err(std::io::Error::new(std::io::ErrorKind::NotFound, "No passive listener"))
                        }
                    } else {
                        TcpStream::connect(format!("{}:{}", &remote_ip, port))
                    };
                    
                    if let Ok(mut data_stream) = data_result {
                        let path = Path::new(&cwd);
                        if let Ok(entries) = std::fs::read_dir(path) {
                            for entry in entries.flatten() {
                                if let Ok(metadata) = entry.metadata() {
                                    let name = entry.file_name().to_string_lossy().to_string();
                                    let facts = build_mlst_facts(&metadata);
                                    let line = format!("{}; {}\r\n", facts, name);
                                    let _ = data_stream.write_all(line.as_bytes());
                                }
                            }
                        }
                    }
                    
                    if passive_mode {
                        let mut listeners = passive_listeners.lock().unwrap();
                        listeners.remove(&port);
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
                        
                        if let Some(user) = user
                            && !user.permissions.can_read {
                                stream.write_all(b"550 Permission denied\r\n")?;
                                continue;
                            }
                    }
                    
                    let file_size = std::fs::metadata(&file_path)?.len();
                    let remaining = if rest_offset > 0 && rest_offset < file_size {
                        file_size - rest_offset
                    } else {
                        file_size
                    };
                    
                    stream.write_all(format!("150 Opening BINARY mode data connection ({} bytes)\r\n", remaining).as_bytes())?;
                    
                    if let Some(port) = data_port {
                        let data_result = if passive_mode {
                            let listener_arc = {
                                let listeners = passive_listeners.lock().unwrap();
                                listeners.get(&port).cloned()
                            };
                            
                            if let Some(listener_arc) = listener_arc {
                                let mut listener_guard = listener_arc.lock().unwrap();
                                if let Some(listener) = listener_guard.take() {
                                    listener.set_nonblocking(false)?;
                                    listener.accept().map(|(s, _)| s)
                                } else {
                                    Err(std::io::Error::new(std::io::ErrorKind::NotFound, "No passive listener"))
                                }
                            } else {
                                Err(std::io::Error::new(std::io::ErrorKind::NotFound, "No passive listener"))
                            }
                        } else {
                            TcpStream::connect(format!("{}:{}", &remote_ip, port))
                        };
                        
                        if let Ok(mut data_stream) = data_result
                            && let Ok(mut file) = std::fs::File::open(&file_path) {
                                use std::io::Seek;
                                if rest_offset > 0 {
                                    let _ = file.seek(std::io::SeekFrom::Start(rest_offset));
                                }
                                
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
                        
                        if passive_mode {
                            let mut listeners = passive_listeners.lock().unwrap();
                            listeners.remove(&port);
                        }
                    }
                    
                    stream.write_all(b"226 Transfer complete\r\n")?;
                    
                    logger.lock().unwrap().client_action("FTP", 
                        &format!("Downloaded: {} ({} bytes from offset {})", filename, remaining, rest_offset),
                        &remote_ip, 
                        current_user.as_deref(), "DOWNLOAD");
                    
                    rest_offset = 0;
                }
            }
            
            "STOR" => {
                if !authenticated {
                    stream.write_all(b"530 Not logged in\r\n")?;
                    continue;
                }
                
                if let Some(filename) = arg {
                    {
                        let users = user_manager.lock().unwrap();
                        let user = current_user.as_ref().and_then(|u| users.get_user(u));
                        
                        if let Some(user) = user
                            && !user.permissions.can_write {
                                stream.write_all(b"550 Permission denied\r\n")?;
                                continue;
                            }
                    }
                    
                    let file_path = Path::new(&cwd).join(filename);
                    stream.write_all(b"150 Opening BINARY mode data connection\r\n")?;
                    
                    if let Some(port) = data_port {
                        let data_result = if passive_mode {
                            let listener_arc = {
                                let listeners = passive_listeners.lock().unwrap();
                                listeners.get(&port).cloned()
                            };
                            
                            if let Some(listener_arc) = listener_arc {
                                let mut listener_guard = listener_arc.lock().unwrap();
                                if let Some(listener) = listener_guard.take() {
                                    listener.set_nonblocking(false)?;
                                    listener.accept().map(|(s, _)| s)
                                } else {
                                    Err(std::io::Error::new(std::io::ErrorKind::NotFound, "No passive listener"))
                                }
                            } else {
                                Err(std::io::Error::new(std::io::ErrorKind::NotFound, "No passive listener"))
                            }
                        } else {
                            TcpStream::connect(format!("{}:{}", &remote_ip, port))
                        };
                        
                        if let Ok(mut data_stream) = data_result {
                            let file_result = if rest_offset > 0 {
                                std::fs::OpenOptions::new()
                                    .write(true)
                                    .create(true)
                                    .truncate(false)
                                    .open(&file_path)
                            } else {
                                std::fs::File::create(&file_path)
                            };
                            
                            if let Ok(mut file) = file_result {
                                use std::io::Seek;
                                if rest_offset > 0 {
                                    let _ = file.seek(std::io::SeekFrom::Start(rest_offset));
                                }
                                
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
                        
                        if passive_mode {
                            let mut listeners = passive_listeners.lock().unwrap();
                            listeners.remove(&port);
                        }
                    }
                    
                    stream.write_all(b"226 Transfer complete\r\n")?;
                    
                    logger.lock().unwrap().client_action("FTP", 
                        &format!("Uploaded: {} at offset {}", filename, rest_offset),
                        &remote_ip, 
                        current_user.as_deref(), "UPLOAD");
                    
                    rest_offset = 0;
                }
            }
            
            "APPE" => {
                if !authenticated {
                    stream.write_all(b"530 Not logged in\r\n")?;
                    continue;
                }
                
                if let Some(filename) = arg {
                    {
                        let users = user_manager.lock().unwrap();
                        let user = current_user.as_ref().and_then(|u| users.get_user(u));
                        
                        if let Some(user) = user
                            && !user.permissions.can_append {
                                stream.write_all(b"550 Permission denied\r\n")?;
                                continue;
                            }
                    }
                    
                    let file_path = Path::new(&cwd).join(filename);
                    stream.write_all(b"150 Opening BINARY mode data connection for append\r\n")?;
                    
                    if let Some(port) = data_port {
                        let data_result = if passive_mode {
                            let listener_arc = {
                                let listeners = passive_listeners.lock().unwrap();
                                listeners.get(&port).cloned()
                            };
                            
                            if let Some(listener_arc) = listener_arc {
                                let mut listener_guard = listener_arc.lock().unwrap();
                                if let Some(listener) = listener_guard.take() {
                                    listener.set_nonblocking(false)?;
                                    listener.accept().map(|(s, _)| s)
                                } else {
                                    Err(std::io::Error::new(std::io::ErrorKind::NotFound, "No passive listener"))
                                }
                            } else {
                                Err(std::io::Error::new(std::io::ErrorKind::NotFound, "No passive listener"))
                            }
                        } else {
                            TcpStream::connect(format!("{}:{}", &remote_ip, port))
                        };
                        
                        if let Ok(mut data_stream) = data_result
                            && let Ok(mut file) = std::fs::OpenOptions::new()
                                
                                .append(true)
                                .create(true)
                                .open(&file_path) {
                                
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
                        
                        if passive_mode {
                            let mut listeners = passive_listeners.lock().unwrap();
                            listeners.remove(&port);
                        }
                    }
                    
                    stream.write_all(b"226 Transfer complete\r\n")?;
                    
                    logger.lock().unwrap().client_action("FTP", 
                        &format!("Appended: {}", filename),
                        &remote_ip, 
                        current_user.as_deref(), "APPEND");
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
                    
                    if let Some(user) = user
                        && !user.permissions.can_delete {
                            stream.write_all(b"550 Permission denied\r\n")?;
                            continue;
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
                    
                    if let Some(user) = user
                        && !user.permissions.can_mkdir {
                            stream.write_all(b"550 Permission denied\r\n")?;
                            continue;
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
                    
                    if let Some(user) = user
                        && !user.permissions.can_rmdir {
                            stream.write_all(b"550 Permission denied\r\n")?;
                            continue;
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
            
            "RNFR" => {
                if !authenticated {
                    stream.write_all(b"530 Not logged in\r\n")?;
                    continue;
                }
                
                {
                    let users = user_manager.lock().unwrap();
                    let user = current_user.as_ref().and_then(|u| users.get_user(u));
                    
                    if let Some(user) = user
                        && !user.permissions.can_rename {
                            stream.write_all(b"550 Permission denied\r\n")?;
                            continue;
                        }
                }
                
                stream.write_all(b"350 File exists, ready for destination name\r\n")?;
            }
            
            "RNTO" => {
                stream.write_all(b"250 Rename successful\r\n")?;
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
                if let Some(filename) = arg {
                    let file_path = Path::new(&cwd).join(filename);
                    if let Ok(metadata) = std::fs::metadata(&file_path) {
                        let mtime = get_file_mtime_raw(&metadata);
                        stream.write_all(format!("213 {}\r\n", mtime).as_bytes())?;
                    } else {
                        stream.write_all(b"550 File not found\r\n")?;
                    }
                }
            }
            
            "NOOP" => {
                stream.write_all(b"200 OK\r\n")?;
            }
            
            "STAT" => {
                stream.write_all(b"211 FTP server status\r\n")?;
            }
            
            "ABOR" => {
                stream.write_all(b"226 Abort successful\r\n")?;
            }
            
            _ => {
                stream.write_all(b"202 Command not implemented\r\n")?;
            }
        }
    }
    
    Ok(())
}

#[allow(dead_code)]
struct TlsContext {
    cert_path: String,
    key_path: String,
}

fn create_tls_context(cert_path: &str, key_path: &str) -> Result<TlsContext> {
    Ok(TlsContext {
        cert_path: cert_path.to_string(),
        key_path: key_path.to_string(),
    })
}

fn find_available_passive_port(passive_listeners: &Arc<Mutex<HashMap<u16, Arc<Mutex<Option<TcpListener>>>>>>, 
                               port_min: u16, port_max: u16) -> Result<u16> {
    let listeners = passive_listeners.lock().unwrap();
    
    for port in port_min..=port_max {
        if !listeners.contains_key(&port) {
            return Ok(port);
        }
    }
    
    anyhow::bail!("No available passive ports in range {}-{}", port_min, port_max)
}

fn get_file_mtime(metadata: &std::fs::Metadata) -> String {
    use std::time::UNIX_EPOCH;
    
    if let Ok(time) = metadata.modified()
        && let Ok(duration) = time.duration_since(UNIX_EPOCH) {
            let secs = duration.as_secs();
            let days = secs / 86400;
            let years = 1970 + days / 365;
            let remaining_days = days % 365;
            let months = remaining_days / 30 + 1;
            let day = remaining_days % 30 + 1;
            let hour = (secs % 86400) / 3600;
            let minute = (secs % 3600) / 60;
            return format!("{:04}-{:02}-{:02} {:02}:{:02}", years, months, day, hour, minute);
        }
    "Jan 01 00:00".to_string()
}

fn get_file_mtime_raw(metadata: &std::fs::Metadata) -> String {
    use std::time::UNIX_EPOCH;
    
    if let Ok(time) = metadata.modified()
        && let Ok(duration) = time.duration_since(UNIX_EPOCH) {
            return format!("{}", duration.as_secs());
        }
    "0".to_string()
}

fn build_mlst_facts(metadata: &std::fs::Metadata) -> String {
    let mut facts: Vec<String> = Vec::new();
    
    if metadata.is_dir() {
        facts.push("type=dir".to_string());
    } else {
        facts.push("type=file".to_string());
    }
    
    facts.push(format!("size={}", metadata.len()));
    
    if let Ok(time) = metadata.modified()
        && let Ok(duration) = time.duration_since(std::time::UNIX_EPOCH) {
            facts.push(format!("modify={}", duration.as_secs()));
        }
    
    facts.join("; ")
}
