use anyhow::Result;
use std::net::TcpListener;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::config::Config;
use crate::users::UserManager;
use crate::logger::Logger;

pub struct SftpServer {
    config: Arc<Mutex<Config>>,
    user_manager: Arc<Mutex<UserManager>>,
    logger: Arc<Mutex<Logger>>,
    running: Arc<Mutex<bool>>,
}

impl SftpServer {
    pub fn new(config: Arc<Mutex<Config>>, user_manager: Arc<Mutex<UserManager>>, 
               logger: Arc<Mutex<Logger>>) -> Self {
        SftpServer {
            config,
            user_manager,
            logger,
            running: Arc::new(Mutex::new(false)),
        }
    }
    
    pub fn start(&self) -> Result<()> {
        let (bind_ip, sftp_port, host_key_path) = {
            let cfg = self.config.lock().unwrap();
            (cfg.server.bind_ip.clone(), cfg.server.sftp_port, cfg.sftp.host_key_path.clone())
        };
        let bind_addr = format!("{}:{}", bind_ip, sftp_port);
        let listener = TcpListener::bind(&bind_addr)?;
        
        {
            let mut running = self.running.lock().unwrap();
            *running = true;
        }
        
        let config = Arc::clone(&self.config);
        let user_manager = Arc::clone(&self.user_manager);
        let logger = Arc::clone(&self.logger);
        let running = Arc::clone(&self.running);
        
        let host_key = generate_or_load_host_key(&host_key_path)?;
        
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
                        let host_key = host_key.clone();
                        
                        std::thread::spawn(move || {
                            if let Err(e) = handle_ssh_connection(stream, &config, &user_manager, &logger, &host_key) {
                                eprintln!("SSH connection error: {}", e);
                            }
                        });
                    }
                    Err(e) => {
                        eprintln!("Failed to accept SSH connection: {}", e);
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

use std::io::{Read, Write};
use std::fs::File;

#[derive(Clone)]
#[allow(dead_code)]
struct HostKey {
    rsa_private: Vec<u8>,
    rsa_public: Vec<u8>,
}

fn generate_or_load_host_key(path: &str) -> Result<HostKey> {
    use rand::rngs::OsRng;
    use rsa::{RsaPrivateKey, RsaPublicKey};
    use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
    
    let path = Path::new(path);
    
    if path.exists() {
        let private_key = std::fs::read(path)?;
        let public_path = path.with_extension("pub");
        let public_key = if public_path.exists() {
            std::fs::read(&public_path)?
        } else {
            vec![]
        };
        return Ok(HostKey {
            rsa_private: private_key,
            rsa_public: public_key,
        });
    }
    
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    
    let mut rng = OsRng;
    let bits = 2048;
    let private_key = RsaPrivateKey::new(&mut rng, bits)?;
    let public_key = RsaPublicKey::from(&private_key);
    
    let private_pem = private_key.to_pkcs8_pem(LineEnding::LF)?;
    let public_pem = public_key.to_public_key_pem(LineEnding::LF)?;
    
    std::fs::write(path, private_pem.as_bytes())?;
    let public_path = path.with_extension("pub");
    std::fs::write(&public_path, public_pem.as_bytes())?;
    
    Ok(HostKey {
        rsa_private: private_pem.as_bytes().to_vec(),
        rsa_public: public_pem.as_bytes().to_vec(),
    })
}

fn handle_ssh_connection(mut stream: std::net::TcpStream,
                         config: &Arc<Mutex<Config>>,
                         user_manager: &Arc<Mutex<UserManager>>,
                         logger: &Arc<Mutex<Logger>>,
                         _host_key: &HostKey) -> Result<()> {
    
    let remote_addr = stream.peer_addr()?;
    let remote_ip = remote_addr.ip().to_string();
    
    {
        let cfg = config.lock().unwrap();
        if !cfg.is_ip_allowed(&remote_ip) {
            return Ok(());
        }
    }
    
    let auth_timeout = config.lock().unwrap().sftp.auth_timeout;
    stream.set_read_timeout(Some(Duration::from_secs(auth_timeout)))?;
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;
    
    let server_version = b"SSH-2.0-WFTPG_SFTP_2.0\r\n";
    stream.write_all(server_version)?;
    
    let mut client_version_buf = [0u8; 256];
    let version_len = stream.read(&mut client_version_buf)?;
    let client_version = String::from_utf8_lossy(&client_version_buf[..version_len]).to_string();
    
    logger.lock().unwrap().info("SFTP", &format!("Client: {}", client_version.trim()));
    
    let mut session_keys = SessionKeys::new();
    
    let mut kex_done = false;
    let mut authenticated = false;
    let mut current_user: Option<String> = None;
    let mut home_dir = config.lock().unwrap().sftp.default_home.clone();
    let mut sftp_initialized = false;
    let _rest_offset: u64 = 0;
    
    loop {
        let mut packet_len_buf = [0u8; 4];
        match stream.read_exact(&mut packet_len_buf) {
            Ok(_) => {},
            Err(_) => break,
        }
        
        let packet_len = u32::from_be_bytes(packet_len_buf) as usize;
        if packet_len == 0 || packet_len > 1024 * 1024 {
            break;
        }
        
        let padding_len;
        {
            let mut first_byte = [0u8; 1];
            stream.read_exact(&mut first_byte)?;
            padding_len = first_byte[0] as usize;
        }
        
        let payload_len = packet_len - 1 - padding_len;
        let mut payload_buf = vec![0u8; payload_len];
        stream.read_exact(&mut payload_buf)?;
        
        let mut padding_buf = vec![0u8; padding_len];
        stream.read_exact(&mut padding_buf)?;
        
        let msg_type = payload_buf.first().copied().unwrap_or(0);
        
        if !kex_done {
            match msg_type {
                20 => {
                    let response = handle_kex_init(&mut session_keys, &payload_buf)?;
                    send_ssh_packet(&mut stream, &response)?;
                }
                
                30 => {
                    handle_kex_dh(&mut stream, &mut session_keys, &payload_buf)?;
                    kex_done = true;
                    
                    let new_keys = build_new_keys_response();
                    send_ssh_packet(&mut stream, &new_keys)?;
                }
                
                _ => {}
            }
        } else if !authenticated {
            match msg_type {
                50 => {
                    let username = extract_ssh_string(&payload_buf, 1);
                    let method = extract_ssh_string(&payload_buf, username.len() + 5);
                    
                    if method == "password" {
                        let password = extract_ssh_string(&payload_buf, username.len() + method.len() + 9);
                        
                        let mut users = user_manager.lock().unwrap();
                        match users.authenticate(&username, &password) {
                            Ok(true) => {
                                authenticated = true;
                                current_user = Some(username.clone());
                                if let Some(user) = users.get_user(&username) {
                                    home_dir = user.home_dir.clone();
                                }
                                
                                let response = build_ssh_msg_userauth_success();
                                send_ssh_packet(&mut stream, &response)?;
                                
                                logger.lock().unwrap().client_action("SFTP",
                                    &format!("User {} logged in", username),
                                    &remote_ip,
                                    Some(&username), "LOGIN");
                            }
                            _ => {
                                let response = build_ssh_msg_userauth_failure();
                                send_ssh_packet(&mut stream, &response)?;
                            }
                        }
                    } else if method == "publickey" {
                        let response = build_ssh_msg_userauth_failure();
                        send_ssh_packet(&mut stream, &response)?;
                    } else {
                        let response = build_ssh_msg_userauth_pk_ok();
                        send_ssh_packet(&mut stream, &response)?;
                    }
                }
                
                1 => {
                    if !authenticated {
                        break;
                    }
                    
                    let version = parse_u32(&payload_buf, 1);
                    let response = build_sftp_version_response(version.min(6));
                    send_ssh_packet(&mut stream, &response)?;
                    sftp_initialized = true;
                }
                
                _ => {}
            }
        } else if sftp_initialized {
            match msg_type {
                1 => {
                    let version = parse_u32(&payload_buf, 1);
                    let response = build_sftp_version_response(version.min(6));
                    send_ssh_packet(&mut stream, &response)?;
                }
                
                3 => {
                    let id = parse_u32(&payload_buf, 1);
                    let path_len = parse_u32(&payload_buf, 5) as usize;
                    let path = String::from_utf8_lossy(&payload_buf[9..9+path_len]).to_string();
                    
                    let full_path = resolve_path(&home_dir, &path);
                    
                    if full_path.exists() && full_path.is_dir() {
                        let handle = format!("dir_{:08x}", id);
                        let response = build_sftp_handle_response(id, &handle);
                        send_ssh_packet(&mut stream, &response)?;
                    } else {
                        let response = build_sftp_status_response(id, 2, "No such directory", "");
                        send_ssh_packet(&mut stream, &response)?;
                    }
                }
                
                4 => {
                    let id = parse_u32(&payload_buf, 1);
                    let handle_len = parse_u32(&payload_buf, 5) as usize;
                    let _handle = String::from_utf8_lossy(&payload_buf[9..9+handle_len]).to_string();
                    
                    let response = build_sftp_status_response(id, 0, "OK", "");
                    send_ssh_packet(&mut stream, &response)?;
                }
                
                5 => {
                    let id = parse_u32(&payload_buf, 1);
                    let path_len = parse_u32(&payload_buf, 5) as usize;
                    let path = String::from_utf8_lossy(&payload_buf[9..9+path_len]).to_string();
                    
                    let full_path = resolve_path(&home_dir, &path);
                    
                    match std::fs::read_dir(&full_path) {
                        Ok(entries) => {
                            let mut names = Vec::new();
                            let mut attrs_list = Vec::new();
                            
                            for entry in entries.flatten() {
                                let name = entry.file_name().to_string_lossy().to_string();
                                let long_name = format_long_name(&entry.path());
                                names.push((name, long_name));
                                
                                if let Ok(metadata) = entry.metadata() {
                                    attrs_list.push(build_file_attrs(&metadata));
                                } else {
                                    attrs_list.push(vec![0u8; 32]);
                                }
                            }
                            
                            let response = build_sftp_name_response_extended(id, &names, &attrs_list);
                            send_ssh_packet(&mut stream, &response)?;
                            
                            logger.lock().unwrap().client_action("SFTP",
                                &format!("Listed directory: {}", path),
                                &remote_ip,
                                current_user.as_deref(), "LIST");
                        }
                        Err(_) => {
                            let response = build_sftp_status_response(id, 2, "No such file", "");
                            send_ssh_packet(&mut stream, &response)?;
                        }
                    }
                }
                
                11 => {
                    let id = parse_u32(&payload_buf, 1);
                    let path_len = parse_u32(&payload_buf, 5) as usize;
                    let path = String::from_utf8_lossy(&payload_buf[9..9+path_len]).to_string();
                    
                    let offset_pos = 9 + path_len;
                    let offset = parse_u64(&payload_buf, offset_pos);
                    let len_pos = offset_pos + 8;
                    let length = parse_u32(&payload_buf, len_pos) as usize;
                    
                    let full_path = resolve_path(&home_dir, &path);
                    
                    {
                        let users = user_manager.lock().unwrap();
                        if let Some(user) = current_user.as_ref().and_then(|u| users.get_user(u))
                            && !user.permissions.can_read {
                                let response = build_sftp_status_response(id, 3, "Permission denied", "");
                                send_ssh_packet(&mut stream, &response)?;
                                continue;
                            }
                    }
                    
                    match File::open(&full_path) {
                        Ok(mut file) => {
                            use std::io::Seek;
                            if offset > 0 {
                                let _ = file.seek(std::io::SeekFrom::Start(offset));
                            }
                            
                            let mut content = vec![0u8; length.min(32768)];
                            match file.read(&mut content) {
                                Ok(n) => {
                                    content.truncate(n);
                                    let response = build_sftp_data_response(id, &content);
                                    send_ssh_packet(&mut stream, &response)?;
                                    
                                    logger.lock().unwrap().client_action("SFTP",
                                        &format!("Downloaded: {} ({} bytes from offset {})", path, n, offset),
                                        &remote_ip,
                                        current_user.as_deref(), "DOWNLOAD");
                                }
                                Err(_) => {
                                    let response = build_sftp_status_response(id, 4, "Read error", "");
                                    send_ssh_packet(&mut stream, &response)?;
                                }
                            }
                        }
                        Err(_) => {
                            let response = build_sftp_status_response(id, 2, "No such file", "");
                            send_ssh_packet(&mut stream, &response)?;
                        }
                    }
                }
                
                6 => {
                    let id = parse_u32(&payload_buf, 1);
                    let path_len = parse_u32(&payload_buf, 5) as usize;
                    let path = String::from_utf8_lossy(&payload_buf[9..9+path_len]).to_string();
                    
                    let offset_pos = 9 + path_len;
                    let offset = parse_u64(&payload_buf, offset_pos);
                    let len_pos = offset_pos + 8;
                    let data_len = parse_u32(&payload_buf, len_pos) as usize;
                    let data_start = len_pos + 4;
                    let data = &payload_buf[data_start..data_start + data_len];
                    
                    {
                        let users = user_manager.lock().unwrap();
                        if let Some(user) = current_user.as_ref().and_then(|u| users.get_user(u))
                            && !user.permissions.can_write {
                                let response = build_sftp_status_response(id, 3, "Permission denied", "");
                                send_ssh_packet(&mut stream, &response)?;
                                continue;
                            }
                    }
                    
                    let full_path = resolve_path(&home_dir, &path);
                    
                    let result = if offset == 0 {
                        std::fs::write(&full_path, data)
                    } else {
                        File::open(&full_path).and_then(|mut f| {
                            use std::io::Seek;
                            f.seek(std::io::SeekFrom::Start(offset))?;
                            f.write_all(data)
                        })
                    };
                    
                    match result {
                        Ok(_) => {
                            let response = build_sftp_status_response(id, 0, "OK", "");
                            send_ssh_packet(&mut stream, &response)?;
                            
                            logger.lock().unwrap().client_action("SFTP",
                                &format!("Uploaded: {} ({} bytes at offset {})", path, data_len, offset),
                                &remote_ip,
                                current_user.as_deref(), "UPLOAD");
                        }
                        Err(_) => {
                            let response = build_sftp_status_response(id, 4, "Write error", "");
                            send_ssh_packet(&mut stream, &response)?;
                        }
                    }
                }
                
                13 => {
                    let id = parse_u32(&payload_buf, 1);
                    let path_len = parse_u32(&payload_buf, 5) as usize;
                    let path = String::from_utf8_lossy(&payload_buf[9..9+path_len]).to_string();
                    
                    {
                        let users = user_manager.lock().unwrap();
                        if let Some(user) = current_user.as_ref().and_then(|u| users.get_user(u))
                            && !user.permissions.can_delete {
                                let response = build_sftp_status_response(id, 3, "Permission denied", "");
                                send_ssh_packet(&mut stream, &response)?;
                                continue;
                            }
                    }
                    
                    let full_path = resolve_path(&home_dir, &path);
                    
                    match std::fs::remove_file(&full_path) {
                        Ok(_) => {
                            let response = build_sftp_status_response(id, 0, "OK", "");
                            send_ssh_packet(&mut stream, &response)?;
                            
                            logger.lock().unwrap().client_action("SFTP",
                                &format!("Deleted: {}", path),
                                &remote_ip,
                                current_user.as_deref(), "DELETE");
                        }
                        Err(_) => {
                            let response = build_sftp_status_response(id, 2, "Delete failed", "");
                            send_ssh_packet(&mut stream, &response)?;
                        }
                    }
                }
                
                14 => {
                    let id = parse_u32(&payload_buf, 1);
                    let path_len = parse_u32(&payload_buf, 5) as usize;
                    let path = String::from_utf8_lossy(&payload_buf[9..9+path_len]).to_string();
                    
                    {
                        let users = user_manager.lock().unwrap();
                        if let Some(user) = current_user.as_ref().and_then(|u| users.get_user(u))
                            && !user.permissions.can_mkdir {
                                let response = build_sftp_status_response(id, 3, "Permission denied", "");
                                send_ssh_packet(&mut stream, &response)?;
                                continue;
                            }
                    }
                    
                    let full_path = resolve_path(&home_dir, &path);
                    
                    match std::fs::create_dir_all(&full_path) {
                        Ok(_) => {
                            let response = build_sftp_status_response(id, 0, "OK", "");
                            send_ssh_packet(&mut stream, &response)?;
                            
                            logger.lock().unwrap().client_action("SFTP",
                                &format!("Created directory: {}", path),
                                &remote_ip,
                                current_user.as_deref(), "MKDIR");
                        }
                        Err(_) => {
                            let response = build_sftp_status_response(id, 4, "Create failed", "");
                            send_ssh_packet(&mut stream, &response)?;
                        }
                    }
                }
                
                15 => {
                    let id = parse_u32(&payload_buf, 1);
                    let path_len = parse_u32(&payload_buf, 5) as usize;
                    let path = String::from_utf8_lossy(&payload_buf[9..9+path_len]).to_string();
                    
                    {
                        let users = user_manager.lock().unwrap();
                        if let Some(user) = current_user.as_ref().and_then(|u| users.get_user(u))
                            && !user.permissions.can_rmdir {
                                let response = build_sftp_status_response(id, 3, "Permission denied", "");
                                send_ssh_packet(&mut stream, &response)?;
                                continue;
                            }
                    }
                    
                    let full_path = resolve_path(&home_dir, &path);
                    
                    match std::fs::remove_dir_all(&full_path) {
                        Ok(_) => {
                            let response = build_sftp_status_response(id, 0, "OK", "");
                            send_ssh_packet(&mut stream, &response)?;
                            
                            logger.lock().unwrap().client_action("SFTP",
                                &format!("Removed directory: {}", path),
                                &remote_ip,
                                current_user.as_deref(), "RMDIR");
                        }
                        Err(_) => {
                            let response = build_sftp_status_response(id, 4, "Remove failed", "");
                            send_ssh_packet(&mut stream, &response)?;
                        }
                    }
                }
                
                16 => {
                    let id = parse_u32(&payload_buf, 1);
                    let old_path_len = parse_u32(&payload_buf, 5) as usize;
                    let old_path = String::from_utf8_lossy(&payload_buf[9..9+old_path_len]).to_string();
                    
                    let new_path_start = 9 + old_path_len + 4;
                    let new_path_len = parse_u32(&payload_buf, 9 + old_path_len) as usize;
                    let new_path = String::from_utf8_lossy(&payload_buf[new_path_start..new_path_start+new_path_len]).to_string();
                    
                    {
                        let users = user_manager.lock().unwrap();
                        if let Some(user) = current_user.as_ref().and_then(|u| users.get_user(u))
                            && !user.permissions.can_rename {
                                let response = build_sftp_status_response(id, 3, "Permission denied", "");
                                send_ssh_packet(&mut stream, &response)?;
                                continue;
                            }
                    }
                    
                    let old_full = resolve_path(&home_dir, &old_path);
                    let new_full = resolve_path(&home_dir, &new_path);
                    
                    match std::fs::rename(&old_full, &new_full) {
                        Ok(_) => {
                            let response = build_sftp_status_response(id, 0, "OK", "");
                            send_ssh_packet(&mut stream, &response)?;
                            
                            logger.lock().unwrap().client_action("SFTP",
                                &format!("Renamed: {} -> {}", old_path, new_path),
                                &remote_ip,
                                current_user.as_deref(), "RENAME");
                        }
                        Err(_) => {
                            let response = build_sftp_status_response(id, 4, "Rename failed", "");
                            send_ssh_packet(&mut stream, &response)?;
                        }
                    }
                }
                
                17 => {
                    let id = parse_u32(&payload_buf, 1);
                    let path_len = parse_u32(&payload_buf, 5) as usize;
                    let path = String::from_utf8_lossy(&payload_buf[9..9+path_len]).to_string();
                    
                    let full_path = resolve_path(&home_dir, &path);
                    
                    match std::fs::metadata(&full_path) {
                        Ok(metadata) => {
                            let attrs = build_file_attrs(&metadata);
                            let response = build_sftp_attrs_response(id, &attrs);
                            send_ssh_packet(&mut stream, &response)?;
                        }
                        Err(_) => {
                            let response = build_sftp_status_response(id, 2, "No such file", "");
                            send_ssh_packet(&mut stream, &response)?;
                        }
                    }
                }
                
                _ => {
                    let response = build_sftp_status_response(0, 8, "Unsupported", "");
                    send_ssh_packet(&mut stream, &response)?;
                }
            }
        } else {
            if msg_type == 1 {
                let version = parse_u32(&payload_buf, 1);
                let response = build_sftp_version_response(version.min(6));
                send_ssh_packet(&mut stream, &response)?;
                sftp_initialized = true;
            }
        }
    }
    
    Ok(())
}

#[allow(dead_code)]
struct SessionKeys {
    session_id: Option<Vec<u8>>,
    client_kex_data: Vec<u8>,
    server_kex_data: Vec<u8>,
}

impl SessionKeys {
    fn new() -> Self {
        SessionKeys {
            session_id: None,
            client_kex_data: Vec::new(),
            server_kex_data: Vec::new(),
        }
    }
}

fn handle_kex_init(session_keys: &mut SessionKeys, payload: &[u8]) -> Result<Vec<u8>> {
    session_keys.client_kex_data = payload.to_vec();
    
    let mut server_kex = vec![20u8];
    server_kex.extend_from_slice(&[0, 0, 0, 64]);
    server_kex.extend_from_slice(b"curve25519-sha256,diffie-hellman-group14-sha256");
    server_kex.extend_from_slice(&[0, 0, 0, 44]);
    server_kex.extend_from_slice(b"rsa-sha2-256,ssh-ed25519");
    server_kex.extend_from_slice(&[0, 0, 0, 52]);
    server_kex.extend_from_slice(b"aes256-ctr,aes256-gcm@openssh.com,chacha20-poly1305");
    server_kex.extend_from_slice(&[0, 0, 0, 52]);
    server_kex.extend_from_slice(b"aes256-ctr,aes256-gcm@openssh.com,chacha20-poly1305");
    server_kex.extend_from_slice(&[0, 0, 0, 24]);
    server_kex.extend_from_slice(b"hmac-sha2-256,hmac-sha2-512");
    server_kex.extend_from_slice(&[0, 0, 0, 24]);
    server_kex.extend_from_slice(b"hmac-sha2-256,hmac-sha2-512");
    server_kex.extend_from_slice(&[0, 0, 0, 12]);
    server_kex.extend_from_slice(b"none,zlib");
    server_kex.extend_from_slice(&[0, 0, 0, 12]);
    server_kex.extend_from_slice(b"none,zlib");
    server_kex.extend_from_slice(&[0, 0, 0, 0]);
    server_kex.extend_from_slice(&[0, 0, 0, 0]);
    server_kex.extend_from_slice(&[0]);
    server_kex.extend_from_slice(&[0, 0, 0, 0]);
    
    session_keys.server_kex_data = server_kex.clone();
    Ok(server_kex)
}

fn handle_kex_dh(stream: &mut std::net::TcpStream, _session_keys: &mut SessionKeys, _payload: &[u8]) -> Result<()> {
    use rand::rngs::OsRng;
    
    let mut server_kex_reply = vec![31u8];
    
    let public_key_blob = b"ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQC";
    server_kex_reply.extend_from_slice(&(public_key_blob.len() as u32).to_be_bytes());
    server_kex_reply.extend_from_slice(public_key_blob);
    
    let f_bytes: [u8; 32] = {
        let mut rng = OsRng;
        use rand::RngCore;
        let mut bytes = [0u8; 32];
        rng.fill_bytes(&mut bytes);
        bytes
    };
    
    server_kex_reply.extend_from_slice(&(f_bytes.len() as u32).to_be_bytes());
    server_kex_reply.extend_from_slice(&f_bytes);
    
    let mut signature: Vec<u8> = b"\x00\x00\x00\x0cssh-rsa\x00\x00\x00\x40".to_vec();
    signature.extend_from_slice(&[0u8; 64]);
    server_kex_reply.extend_from_slice(&(signature.len() as u32).to_be_bytes());
    server_kex_reply.extend_from_slice(&signature);
    
    send_ssh_packet(stream, &server_kex_reply)?;
    
    Ok(())
}

fn build_new_keys_response() -> Vec<u8> {
    vec![21u8]
}

fn build_ssh_msg_userauth_success() -> Vec<u8> {
    vec![52u8]
}

fn build_ssh_msg_userauth_failure() -> Vec<u8> {
    let mut payload = vec![51u8];
    payload.extend_from_slice(&8u32.to_be_bytes());
    payload.extend_from_slice(b"password");
    payload.push(0);
    payload
}

fn build_ssh_msg_userauth_pk_ok() -> Vec<u8> {
    let mut payload = vec![60u8];
    payload.extend_from_slice(&7u32.to_be_bytes());
    payload.extend_from_slice(b"ssh-rsa");
    payload.extend_from_slice(&0u32.to_be_bytes());
    payload
}

fn send_ssh_packet(stream: &mut std::net::TcpStream, payload: &[u8]) -> Result<()> {
    let padding_len = ((8 - (payload.len() + 5) % 8) % 8) + 4;
    let packet_len = 1 + payload.len() + padding_len;
    
    let mut packet = Vec::new();
    packet.extend_from_slice(&(packet_len as u32).to_be_bytes());
    packet.push(padding_len as u8);
    packet.extend_from_slice(payload);
    packet.extend_from_slice(&vec![0u8; padding_len]);
    
    stream.write_all(&packet)?;
    Ok(())
}

fn resolve_path(home_dir: &str, path: &str) -> std::path::PathBuf {
    if path.starts_with('/') {
        std::path::PathBuf::from(path)
    } else {
        std::path::PathBuf::from(home_dir).join(path)
    }
}

fn format_long_name(path: &std::path::Path) -> String {
    match std::fs::metadata(path) {
        Ok(metadata) => {
            let is_dir = metadata.is_dir();
            let mode = if is_dir { "drwxr-xr-x" } else { "-rw-r--r--" };
            let size = metadata.len();
            let name = path.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            format!("{} 1 user user {:>10} Jan 01 00:00 {}", mode, size, name)
        }
        Err(_) => "?????????? ? ? ? ? ? ?".to_string()
    }
}

fn build_file_attrs(metadata: &std::fs::Metadata) -> Vec<u8> {
    let mut attrs = Vec::new();
    
    let permissions = if metadata.is_dir() { 0o755u32 } else { 0o644u32 };
    attrs.extend_from_slice(&permissions.to_be_bytes());
    
    attrs.extend_from_slice(&0u64.to_be_bytes());
    attrs.extend_from_slice(&0u64.to_be_bytes());
    
    attrs.extend_from_slice(&metadata.len().to_be_bytes());
    
    attrs.extend_from_slice(&0u32.to_be_bytes());
    attrs.extend_from_slice(&0u32.to_be_bytes());
    attrs.extend_from_slice(&0u32.to_be_bytes());
    attrs.extend_from_slice(&0u32.to_be_bytes());
    
    attrs
}

fn parse_u32(data: &[u8], offset: usize) -> u32 {
    if offset + 4 > data.len() {
        return 0;
    }
    u32::from_be_bytes([data[offset], data[offset+1], data[offset+2], data[offset+3]])
}

fn parse_u64(data: &[u8], offset: usize) -> u64 {
    if offset + 8 > data.len() {
        return 0;
    }
    u64::from_be_bytes([
        data[offset], data[offset+1], data[offset+2], data[offset+3],
        data[offset+4], data[offset+5], data[offset+6], data[offset+7]
    ])
}

fn extract_ssh_string(data: &[u8], offset: usize) -> String {
    if offset + 4 > data.len() {
        return String::new();
    }
    let len = parse_u32(data, offset) as usize;
    if offset + 4 + len > data.len() {
        return String::new();
    }
    String::from_utf8_lossy(&data[offset+4..offset+4+len]).to_string()
}

fn build_sftp_packet(payload: &[u8]) -> Vec<u8> {
    let mut packet = Vec::new();
    packet.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    packet.extend_from_slice(payload);
    packet
}

fn build_sftp_version_response(version: u32) -> Vec<u8> {
    let mut payload = vec![2u8];
    payload.extend_from_slice(&version.to_be_bytes());
    build_sftp_packet(&payload)
}

fn build_sftp_handle_response(id: u32, handle: &str) -> Vec<u8> {
    let mut payload = vec![102u8];
    payload.extend_from_slice(&id.to_be_bytes());
    payload.extend_from_slice(&(handle.len() as u32).to_be_bytes());
    payload.extend_from_slice(handle.as_bytes());
    build_sftp_packet(&payload)
}

fn build_sftp_status_response(id: u32, status: u32, msg: &str, lang: &str) -> Vec<u8> {
    let mut payload = vec![101u8];
    payload.extend_from_slice(&id.to_be_bytes());
    payload.extend_from_slice(&status.to_be_bytes());
    payload.extend_from_slice(&(msg.len() as u32).to_be_bytes());
    payload.extend_from_slice(msg.as_bytes());
    payload.extend_from_slice(&(lang.len() as u32).to_be_bytes());
    payload.extend_from_slice(lang.as_bytes());
    build_sftp_packet(&payload)
}

fn build_sftp_data_response(id: u32, data: &[u8]) -> Vec<u8> {
    let mut payload = vec![103u8];
    payload.extend_from_slice(&id.to_be_bytes());
    payload.extend_from_slice(&(data.len() as u32).to_be_bytes());
    payload.extend_from_slice(data);
    build_sftp_packet(&payload)
}

fn build_sftp_name_response_extended(id: u32, names: &[(String, String)], attrs: &[Vec<u8>]) -> Vec<u8> {
    let mut payload = vec![104u8];
    payload.extend_from_slice(&id.to_be_bytes());
    payload.extend_from_slice(&(names.len() as u32).to_be_bytes());
    
    for (i, (name, long_name)) in names.iter().enumerate() {
        payload.extend_from_slice(&(name.len() as u32).to_be_bytes());
        payload.extend_from_slice(name.as_bytes());
        payload.extend_from_slice(&(long_name.len() as u32).to_be_bytes());
        payload.extend_from_slice(long_name.as_bytes());
        
        if i < attrs.len() {
            payload.extend_from_slice(&attrs[i]);
        } else {
            payload.extend_from_slice(&[0u8; 32]);
        }
    }
    
    build_sftp_packet(&payload)
}

fn build_sftp_attrs_response(id: u32, attrs: &[u8]) -> Vec<u8> {
    let mut payload = vec![105u8];
    payload.extend_from_slice(&id.to_be_bytes());
    payload.extend_from_slice(attrs);
    build_sftp_packet(&payload)
}
