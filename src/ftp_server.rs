use anyhow::Result;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::config::Config;
use crate::logger::Logger;
use crate::users::UserManager;

type PassiveListenerMap = Arc<Mutex<HashMap<u16, Arc<Mutex<Option<TcpListener>>>>>>;

pub struct FtpServer {
    config: Arc<Mutex<Config>>,
    user_manager: Arc<Mutex<UserManager>>,
    logger: Arc<Mutex<Logger>>,
    running: Arc<Mutex<bool>>,
    passive_listeners: PassiveListenerMap,
}

impl FtpServer {
    pub fn new(
        config: Arc<Mutex<Config>>,
        user_manager: Arc<Mutex<UserManager>>,
        logger: Arc<Mutex<Logger>>,
    ) -> Self {
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
                            if let Err(_e) = handle_ftp_connection(
                                stream,
                                &config,
                                &user_manager,
                                &logger,
                                &passive_listeners,
                            ) {
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

use std::sync::atomic::{AtomicBool, Ordering};

fn handle_ftp_connection(
    mut stream: TcpStream,
    config: &Arc<Mutex<Config>>,
    user_manager: &Arc<Mutex<UserManager>>,
    logger: &Arc<Mutex<Logger>>,
    passive_listeners: &PassiveListenerMap,
) -> Result<()> {
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
    let mut data_addr: Option<String> = None;
    let mut passive_mode = false;
    let mut cwd;
    {
        let cfg = config.lock().unwrap();
        cwd = cfg.ftp.default_home.clone();
    }

    let mut rest_offset: u64 = 0;
    let mut rename_from: Option<String> = None;
    let abort_flag = Arc::new(AtomicBool::new(false));

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

        let command = String::from_utf8_lossy(&buffer[..bytes_read])
            .trim()
            .to_string();

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
                            logger.lock().unwrap().client_action(
                                "FTP",
                                &format!("User {} logged in", username),
                                &remote_ip,
                                Some(username),
                                "LOGIN",
                            );
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
                stream.write_all(b"211-Features:\r\n SIZE\r\n MDTM\r\n REST STREAM\r\n PASV\r\n EPSV\r\n EPRT\r\n PORT\r\n MLST\r\n MLSD\r\n MODE\r\n STRU\r\n UTF8\r\n TVFS\r\n211 End\r\n")?;
            }

            "HELP" => {
                if let Some(cmd) = arg {
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
                        _ => "214 Unknown command\r\n",
                    };
                    stream.write_all(help_text.as_bytes())?;
                } else {
                    stream.write_all(b"214-The following commands are recognized:\r\n")?;
                    stream.write_all(b"214-USER PASS CWD CDUP PWD LIST NLST RETR STOR\r\n")?;
                    stream.write_all(b"214-DELE MKD RMD RNFR RNTO PASV EPSV PORT EPRT\r\n")?;
                    stream.write_all(b"214-TYPE MODE STRU REST SIZE MDTM ABOR QUIT\r\n")?;
                    stream.write_all(b"214-MLSD MLST SYST FEAT STAT HELP NOOP\r\n")?;
                    stream.write_all(b"214 Direct comments to admin\r\n")?;
                }
            }

            "MODE" => {
                if let Some(mode) = arg {
                    match mode.to_uppercase().as_str() {
                        "S" => {
                            stream.write_all(b"200 Mode set to Stream\r\n")?;
                        }
                        "B" => {
                            stream.write_all(b"504 Block mode not supported\r\n")?;
                        }
                        "C" => {
                            stream.write_all(b"504 Compressed mode not supported\r\n")?;
                        }
                        _ => {
                            stream.write_all(b"501 Unknown mode\r\n")?;
                        }
                    }
                } else {
                    stream.write_all(b"501 Syntax error: MODE requires parameter\r\n")?;
                }
            }

            "STRU" => {
                if let Some(structure) = arg {
                    match structure.to_uppercase().as_str() {
                        "F" => {
                            stream.write_all(b"200 Structure set to File\r\n")?;
                        }
                        "R" => {
                            stream.write_all(b"504 Record structure not supported\r\n")?;
                        }
                        "P" => {
                            stream.write_all(b"504 Page structure not supported\r\n")?;
                        }
                        _ => {
                            stream.write_all(b"501 Unknown structure\r\n")?;
                        }
                    }
                } else {
                    stream.write_all(b"501 Syntax error: STRU requires parameter\r\n")?;
                }
            }

            "ALLO" => {
                stream.write_all(b"200 ALLO command successful\r\n")?;
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
                if let Some(type_code) = arg {
                    match type_code.to_uppercase().as_str() {
                        "I" | "L 8" => {
                            stream.write_all(b"200 Type set to I (Binary)\r\n")?;
                        }
                        "A" | "A N" => {
                            stream.write_all(b"200 Type set to A (ASCII)\r\n")?;
                        }
                        "E" => {
                            stream.write_all(b"200 Type set to E (EBCDIC)\r\n")?;
                        }
                        _ => {
                            stream.write_all(b"501 Unknown type\r\n")?;
                        }
                    }
                } else {
                    stream.write_all(b"200 Type set to I (Binary)\r\n")?;
                }
            }

            "MLST" => {
                if !authenticated {
                    stream.write_all(b"530 Not logged in\r\n")?;
                    continue;
                }

                let target_path = if let Some(path_arg) = arg {
                    if path_arg.starts_with('/') {
                        Path::new(path_arg).to_path_buf()
                    } else {
                        Path::new(&cwd).join(path_arg)
                    }
                } else {
                    Path::new(&cwd).to_path_buf()
                };

                if target_path.exists() {
                    if let Ok(metadata) = target_path.metadata() {
                        let facts = build_mlst_facts(&metadata);
                        let name = target_path.file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| target_path.to_string_lossy().to_string());
                        stream.write_all(format!("250-Listing {}\r\n {}; {}\r\n250 End\r\n", target_path.display(), facts, name).as_bytes())?;
                    } else {
                        stream.write_all(b"550 Failed to get file info\r\n")?;
                    }
                } else {
                    stream.write_all(b"550 File not found\r\n")?;
                }
            }

            "REST" => {
                if let Some(offset_str) = arg {
                    if let Ok(offset) = offset_str.parse::<u64>() {
                        rest_offset = offset;
                        stream.write_all(format!("350 Restarting at {}\r\n", offset).as_bytes())?;
                        logger.lock().unwrap().client_action(
                            "FTP",
                            &format!("REST command: offset {}", offset),
                            &remote_ip,
                            current_user.as_deref(),
                            "REST",
                        );
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
                stream.write_all(
                    format!(
                        "227 Entering Passive Mode ({},{},{})\r\n",
                        ip_octets,
                        passive_port >> 8,
                        passive_port & 0xFF
                    )
                    .as_bytes(),
                )?;

                logger.lock().unwrap().client_action(
                    "FTP",
                    &format!("PASV mode: port {}", passive_port),
                    &remote_ip,
                    current_user.as_deref(),
                    "PASV",
                );
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
                stream.write_all(
                    format!("229 Entering Extended Passive Mode (|||{}|)\r\n", passive_port).as_bytes(),
                )?;
            }

            "PORT" => {
                if let Some(data) = arg {
                    let parts: Vec<u16> = data.split(',').filter_map(|s| s.parse().ok()).collect();
                    if parts.len() == 6 {
                        let port = parts[4] * 256 + parts[5];
                        let addr = format!("{}.{}.{}.{}:{}", parts[0], parts[1], parts[2], parts[3], port);
                        data_port = Some(port);
                        data_addr = Some(addr);
                        passive_mode = false;
                        stream.write_all(b"200 PORT command successful\r\n")?;
                    } else {
                        stream.write_all(b"501 Syntax error in parameters or arguments\r\n")?;
                    }
                } else {
                    stream.write_all(b"501 Syntax error: PORT requires parameters\r\n")?;
                }
            }

            "EPRT" => {
                if let Some(data) = arg {
                    let parts: Vec<&str> = data.split('|').collect();
                    if parts.len() >= 4 {
                        let net_proto = parts[1];
                        let net_addr = parts[2];
                        let tcp_port = parts[3];

                        match net_proto {
                            "1" => {
                                if let Ok(port) = tcp_port.parse::<u16>() {
                                    data_port = Some(port);
                                    data_addr = Some(format!("{}:{}", net_addr, port));
                                    passive_mode = false;
                                    stream.write_all(b"200 EPRT command successful\r\n")?;
                                } else {
                                    stream.write_all(b"501 Invalid port number\r\n")?;
                                }
                            }
                            "2" => {
                                if let Ok(port) = tcp_port.parse::<u16>() {
                                    data_port = Some(port);
                                    data_addr = Some(format!("[{}]:{}", net_addr, port));
                                    passive_mode = false;
                                    stream.write_all(b"200 EPRT command successful (IPv6)\r\n")?;
                                } else {
                                    stream.write_all(b"501 Invalid port number\r\n")?;
                                }
                            }
                            _ => {
                                stream.write_all(b"522 Protocol not supported, use (1,2)\r\n")?;
                            }
                        }
                    } else {
                        stream.write_all(b"501 Syntax error in EPRT parameters\r\n")?;
                    }
                } else {
                    stream.write_all(b"501 Syntax error: EPRT requires parameters\r\n")?;
                }
            }

            "PBSZ" => {
                stream.write_all(b"200 PBSZ=0\r\n")?;
            }

            "PROT" => {
                if let Some(level) = arg {
                    match level.to_uppercase().as_str() {
                        "P" => {
                            stream.write_all(b"200 PROT Private\r\n")?;
                        }
                        "C" => {
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
                                Err(std::io::Error::new(
                                    std::io::ErrorKind::NotFound,
                                    "No passive listener",
                                ))
                            }
                        } else {
                            Err(std::io::Error::new(
                                std::io::ErrorKind::NotFound,
                                "No passive listener",
                            ))
                        }
                    } else if let Some(ref addr) = data_addr {
                        TcpStream::connect(addr)
                    } else {
                        TcpStream::connect(format!("{}:{}", &remote_ip, port))
                    };

                    if let Ok(mut data_stream) = data_result {
                        let path = Path::new(&cwd);
                        if let Ok(entries) = std::fs::read_dir(path) {
                            for entry in entries.flatten() {
                                if let Ok(metadata) = entry.metadata() {
                                    let name = entry.file_name().to_string_lossy().to_string();
                                    let perms = if metadata.is_dir() {
                                        "drwxr-xr-x"
                                    } else {
                                        "-rw-r--r--"
                                    };
                                    let size = metadata.len();
                                    let mtime = get_file_mtime(&metadata);
                                    let line = format!(
                                        "{} 1 user user {:>10} {} {}\r\n",
                                        perms, size, mtime, name
                                    );
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
                                Err(std::io::Error::new(
                                    std::io::ErrorKind::NotFound,
                                    "No passive listener",
                                ))
                            }
                        } else {
                            Err(std::io::Error::new(
                                std::io::ErrorKind::NotFound,
                                "No passive listener",
                            ))
                        }
                    } else if let Some(ref addr) = data_addr {
                        TcpStream::connect(addr)
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

                    stream.write_all(
                        format!("150 Opening BINARY mode data connection ({} bytes)\r\n", remaining)
                            .as_bytes(),
                    )?;

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
                                    Err(std::io::Error::new(
                                        std::io::ErrorKind::NotFound,
                                        "No passive listener",
                                    ))
                                }
                            } else {
                                Err(std::io::Error::new(
                                    std::io::ErrorKind::NotFound,
                                    "No passive listener",
                                ))
                            }
                        } else if let Some(ref addr) = data_addr {
                            TcpStream::connect(addr)
                        } else {
                            TcpStream::connect(format!("{}:{}", &remote_ip, port))
                        };

                        let abort = Arc::clone(&abort_flag);
                        if let Ok(mut data_stream) = data_result
                            && let Ok(mut file) = std::fs::File::open(&file_path) {
                                use std::io::Seek;
                                if rest_offset > 0 {
                                    let _ = file.seek(std::io::SeekFrom::Start(rest_offset));
                                }

                                let mut buf = [0u8; 8192];
                                loop {
                                    if abort.load(Ordering::Relaxed) {
                                        break;
                                    }
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

                    logger.lock().unwrap().client_action(
                        "FTP",
                        &format!(
                            "Downloaded: {} ({} bytes from offset {})",
                            filename, remaining, rest_offset
                        ),
                        &remote_ip,
                        current_user.as_deref(),
                        "DOWNLOAD",
                    );

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
                                    Err(std::io::Error::new(
                                        std::io::ErrorKind::NotFound,
                                        "No passive listener",
                                    ))
                                }
                            } else {
                                Err(std::io::Error::new(
                                    std::io::ErrorKind::NotFound,
                                    "No passive listener",
                                ))
                            }
                        } else if let Some(ref addr) = data_addr {
                            TcpStream::connect(addr)
                        } else {
                            TcpStream::connect(format!("{}:{}", &remote_ip, port))
                        };

                        let abort = Arc::clone(&abort_flag);
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
                                    if abort.load(Ordering::Relaxed) {
                                        break;
                                    }
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

                    logger.lock().unwrap().client_action(
                        "FTP",
                        &format!("Uploaded: {} at offset {}", filename, rest_offset),
                        &remote_ip,
                        current_user.as_deref(),
                        "UPLOAD",
                    );

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
                                    Err(std::io::Error::new(
                                        std::io::ErrorKind::NotFound,
                                        "No passive listener",
                                    ))
                                }
                            } else {
                                Err(std::io::Error::new(
                                    std::io::ErrorKind::NotFound,
                                    "No passive listener",
                                ))
                            }
                        } else if let Some(ref addr) = data_addr {
                            TcpStream::connect(addr)
                        } else {
                            TcpStream::connect(format!("{}:{}", &remote_ip, port))
                        };

                        let abort = Arc::clone(&abort_flag);
                        if let Ok(mut data_stream) = data_result
                            && let Ok(mut file) = std::fs::OpenOptions::new()
                                .append(true)
                                .create(true)
                                .open(&file_path)
                            {
                                let mut buf = [0u8; 8192];
                                loop {
                                    if abort.load(Ordering::Relaxed) {
                                        break;
                                    }
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

                    logger.lock().unwrap().client_action(
                        "FTP",
                        &format!("Appended: {}", filename),
                        &remote_ip,
                        current_user.as_deref(),
                        "APPEND",
                    );
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
                        logger.lock().unwrap().client_action(
                            "FTP",
                            &format!("Deleted: {}", filename),
                            &remote_ip,
                            current_user.as_deref(),
                            "DELETE",
                        );
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
                        logger.lock().unwrap().client_action(
                            "FTP",
                            &format!("Created directory: {}", dirname),
                            &remote_ip,
                            current_user.as_deref(),
                            "MKDIR",
                        );
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
                        logger.lock().unwrap().client_action(
                            "FTP",
                            &format!("Removed directory: {}", dirname),
                            &remote_ip,
                            current_user.as_deref(),
                            "RMDIR",
                        );
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

                if let Some(from_name) = arg {
                    let from_path = Path::new(&cwd).join(from_name);
                    if from_path.exists() {
                        rename_from = Some(from_path.to_string_lossy().to_string());
                        stream.write_all(b"350 File exists, ready for destination name\r\n")?;
                    } else {
                        stream.write_all(b"550 File not found\r\n")?;
                    }
                }
            }

            "RNTO" => {
                if let Some(ref from_path) = rename_from {
                    if let Some(to_name) = arg {
                        let to_path = Path::new(&cwd).join(to_name);
                        if std::fs::rename(from_path, &to_path).is_ok() {
                            stream.write_all(b"250 Rename successful\r\n")?;
                            logger.lock().unwrap().client_action(
                                "FTP",
                                &format!("Renamed: {} -> {}", from_path, to_path.display()),
                                &remote_ip,
                                current_user.as_deref(),
                                "RENAME",
                            );
                        } else {
                            stream.write_all(b"550 Rename failed\r\n")?;
                        }
                    }
                } else {
                    stream.write_all(b"503 Bad sequence of commands\r\n")?;
                }
                rename_from = None;
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
                if let Some(ref username) = current_user {
                    stream.write_all(b"211-FTP server status:\r\n")?;
                    stream.write_all(format!("211-Connected to: {}\r\n", remote_ip).as_bytes())?;
                    stream.write_all(format!("211-Logged in as: {}\r\n", username).as_bytes())?;
                    stream.write_all(format!("211-Current directory: {}\r\n", cwd).as_bytes())?;
                    stream.write_all(format!("211-Transfer mode: {}\r\n", if passive_mode { "Passive" } else { "Active" }).as_bytes())?;
                    stream.write_all(b"211 End\r\n")?;
                } else {
                    stream.write_all(b"211 FTP server status - Not logged in\r\n")?;
                }
            }

            "ABOR" => {
                abort_flag.store(true, Ordering::Relaxed);
                stream.write_all(b"426 Connection closed; transfer aborted\r\n")?;
                stream.write_all(b"226 Abort successful\r\n")?;
            }

            _ => {
                stream.write_all(b"202 Command not implemented\r\n")?;
            }
        }
    }

    Ok(())
}

fn find_available_passive_port(
    passive_listeners: &PassiveListenerMap,
    port_min: u16,
    port_max: u16,
) -> Result<u16> {
    let listeners = passive_listeners.lock().unwrap();

    for port in port_min..=port_max {
        if !listeners.contains_key(&port) {
            return Ok(port);
        }
    }

    anyhow::bail!(
        "No available passive ports in range {}-{}",
        port_min,
        port_max
    )
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
