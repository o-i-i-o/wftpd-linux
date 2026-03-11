use anyhow::Result;
use std::collections::HashMap;
use std::io::{Read, Write};
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
        let bind_ip = self.config.lock().unwrap().server.bind_ip.clone();
        let sftp_port = self.config.lock().unwrap().server.sftp_port;
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
                            if let Err(e) = handle_sftp_connection(stream, &config, &user_manager, &logger) {
                                eprintln!("SFTP connection error: {}", e);
                            }
                        });
                    }
                    Err(e) => {
                        eprintln!("Failed to accept SFTP connection: {}", e);
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

fn handle_sftp_connection(mut stream: std::net::TcpStream, 
                          config: &Arc<Mutex<Config>>,
                          user_manager: &Arc<Mutex<UserManager>>,
                          logger: &Arc<Mutex<Logger>>) -> Result<()> {
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
    
    let ssh_banner = b"SSH-2.0-WFTPG_SFTP_1.0\r\n";
    stream.write_all(ssh_banner)?;
    
    let mut banner_buf = [0u8; 256];
    let banner_len = stream.read(&mut banner_buf)?;
    let _client_banner = String::from_utf8_lossy(&banner_buf[..banner_len]);
    
    let mut authenticated = false;
    let mut current_user: Option<String> = None;
    let mut home_dir = config.lock().unwrap().sftp.default_home.clone();
    let mut sftp_initialized = false;
    
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
        
        let mut packet_buf = vec![0u8; packet_len];
        if stream.read_exact(&mut packet_buf).is_err() {
            break;
        }
        
        let msg_type = packet_buf.first().copied().unwrap_or(0);
        
        if !sftp_initialized {
            match msg_type {
                5 => {
                    let welcome = config.lock().unwrap().ftp.welcome_message.clone();
                    let response = build_ssh_msg_userauth_banner(&welcome);
                    stream.write_all(&response)?;
                }
                
                50 => {
                    if authenticated {
                        continue;
                    }
                    
                    let username = extract_ssh_string(&packet_buf, 1);
                    let method = extract_ssh_string(&packet_buf, username.len() + 5);
                    
                    if method == "password" {
                        let password = extract_ssh_string(&packet_buf, username.len() + method.len() + 9);
                        
                        let mut users = user_manager.lock().unwrap();
                        match users.authenticate(&username, &password) {
                            Ok(true) => {
                                authenticated = true;
                                current_user = Some(username.clone());
                                if let Some(user) = users.get_user(&username) {
                                    home_dir = user.home_dir.clone();
                                }
                                
                                let response = build_ssh_msg_userauth_success();
                                stream.write_all(&response)?;
                                
                                logger.lock().unwrap().client_action("SFTP",
                                    &format!("User {} logged in", username),
                                    &remote_ip,
                                    Some(&username), "LOGIN");
                            }
                            _ => {
                                let response = build_ssh_msg_userauth_failure();
                                stream.write_all(&response)?;
                            }
                        }
                    } else {
                        let response = build_ssh_msg_userauth_failure();
                        stream.write_all(&response)?;
                    }
                }
                
                1 => {
                    if !authenticated {
                        break;
                    }
                    
                    let version = parse_u32(&packet_buf, 1);
                    let response = build_sftp_version_response(version.min(3));
                    stream.write_all(&response)?;
                    sftp_initialized = true;
                }
                
                _ => {}
            }
        } else {
            match msg_type {
                3 => {
                    let id = parse_u32(&packet_buf, 1);
                    let handle = format!("dir_{}", id);
                    let response = build_sftp_handle_response(id, &handle);
                    stream.write_all(&response)?;
                }
                
                4 => {
                    let id = parse_u32(&packet_buf, 1);
                    let response = build_sftp_status_response(id, 0, "OK", "");
                    stream.write_all(&response)?;
                }
                
                5 => {
                    let id = parse_u32(&packet_buf, 1);
                    let path_len = parse_u32(&packet_buf, 5) as usize;
                    let path = String::from_utf8_lossy(&packet_buf[9..9+path_len]).to_string();
                    
                    let full_path = if path.starts_with('/') {
                        path.clone()
                    } else {
                        format!("{}/{}", home_dir, path)
                    };
                    
                    match std::fs::read_dir(&full_path) {
                        Ok(entries) => {
                            let mut names = Vec::new();
                            for entry in entries.flatten() {
                                names.push(entry.file_name().to_string_lossy().to_string());
                            }
                            let response = build_sftp_name_response(id, &names);
                            stream.write_all(&response)?;
                            
                            logger.lock().unwrap().client_action("SFTP",
                                &format!("Listed directory: {}", path),
                                &remote_ip,
                                current_user.as_deref(), "LIST");
                        }
                        Err(_) => {
                            let response = build_sftp_status_response(id, 2, "No such file", "");
                            stream.write_all(&response)?;
                        }
                    }
                }
                
                11 => {
                    let id = parse_u32(&packet_buf, 1);
                    let path_len = parse_u32(&packet_buf, 5) as usize;
                    let path = String::from_utf8_lossy(&packet_buf[9..9+path_len]).to_string();
                    
                    let full_path = if path.starts_with('/') {
                        path.clone()
                    } else {
                        format!("{}/{}", home_dir, path)
                    };
                    
                    match std::fs::File::open(&full_path) {
                        Ok(mut file) => {
                            let mut content = Vec::new();
                            if file.read_to_end(&mut content).is_ok() {
                                let response = build_sftp_data_response(id, &content);
                                stream.write_all(&response)?;
                                
                                logger.lock().unwrap().client_action("SFTP",
                                    &format!("Downloaded: {}", path),
                                    &remote_ip,
                                    current_user.as_deref(), "DOWNLOAD");
                            } else {
                                let response = build_sftp_status_response(id, 4, "Read error", "");
                                stream.write_all(&response)?;
                            }
                        }
                        Err(_) => {
                            let response = build_sftp_status_response(id, 2, "No such file", "");
                            stream.write_all(&response)?;
                        }
                    }
                }
                
                6 => {
                    let id = parse_u32(&packet_buf, 1);
                    let path_len = parse_u32(&packet_buf, 5) as usize;
                    let path = String::from_utf8_lossy(&packet_buf[9..9+path_len]).to_string();
                    
                    let full_path = if path.starts_with('/') {
                        path.clone()
                    } else {
                        format!("{}/{}", home_dir, path)
                    };
                    
                    let data_offset = 9 + path_len + 4;
                    let data_len = parse_u32(&packet_buf, 9 + path_len) as usize;
                    let data = &packet_buf[data_offset..data_offset + data_len];
                    
                    match std::fs::write(&full_path, data) {
                        Ok(_) => {
                            let response = build_sftp_status_response(id, 0, "OK", "");
                            stream.write_all(&response)?;
                            
                            logger.lock().unwrap().client_action("SFTP",
                                &format!("Uploaded: {}", path),
                                &remote_ip,
                                current_user.as_deref(), "UPLOAD");
                        }
                        Err(_) => {
                            let response = build_sftp_status_response(id, 4, "Write error", "");
                            stream.write_all(&response)?;
                        }
                    }
                }
                
                13 => {
                    let id = parse_u32(&packet_buf, 1);
                    let path_len = parse_u32(&packet_buf, 5) as usize;
                    let path = String::from_utf8_lossy(&packet_buf[9..9+path_len]).to_string();
                    
                    let full_path = if path.starts_with('/') {
                        path.clone()
                    } else {
                        format!("{}/{}", home_dir, path)
                    };
                    
                    match std::fs::remove_file(&full_path) {
                        Ok(_) => {
                            let response = build_sftp_status_response(id, 0, "OK", "");
                            stream.write_all(&response)?;
                            
                            logger.lock().unwrap().client_action("SFTP",
                                &format!("Deleted: {}", path),
                                &remote_ip,
                                current_user.as_deref(), "DELETE");
                        }
                        Err(_) => {
                            let response = build_sftp_status_response(id, 2, "Delete failed", "");
                            stream.write_all(&response)?;
                        }
                    }
                }
                
                14 => {
                    let id = parse_u32(&packet_buf, 1);
                    let path_len = parse_u32(&packet_buf, 5) as usize;
                    let path = String::from_utf8_lossy(&packet_buf[9..9+path_len]).to_string();
                    
                    let full_path = if path.starts_with('/') {
                        path.clone()
                    } else {
                        format!("{}/{}", home_dir, path)
                    };
                    
                    match std::fs::create_dir_all(&full_path) {
                        Ok(_) => {
                            let response = build_sftp_status_response(id, 0, "OK", "");
                            stream.write_all(&response)?;
                            
                            logger.lock().unwrap().client_action("SFTP",
                                &format!("Created directory: {}", path),
                                &remote_ip,
                                current_user.as_deref(), "MKDIR");
                        }
                        Err(_) => {
                            let response = build_sftp_status_response(id, 4, "Create failed", "");
                            stream.write_all(&response)?;
                        }
                    }
                }
                
                15 => {
                    let id = parse_u32(&packet_buf, 1);
                    let path_len = parse_u32(&packet_buf, 5) as usize;
                    let path = String::from_utf8_lossy(&packet_buf[9..9+path_len]).to_string();
                    
                    let full_path = if path.starts_with('/') {
                        path.clone()
                    } else {
                        format!("{}/{}", home_dir, path)
                    };
                    
                    match std::fs::remove_dir_all(&full_path) {
                        Ok(_) => {
                            let response = build_sftp_status_response(id, 0, "OK", "");
                            stream.write_all(&response)?;
                            
                            logger.lock().unwrap().client_action("SFTP",
                                &format!("Removed directory: {}", path),
                                &remote_ip,
                                current_user.as_deref(), "RMDIR");
                        }
                        Err(_) => {
                            let response = build_sftp_status_response(id, 4, "Remove failed", "");
                            stream.write_all(&response)?;
                        }
                    }
                }
                
                _ => {
                    let response = build_sftp_status_response(0, 8, "Unsupported", "");
                    stream.write_all(&response)?;
                }
            }
        }
    }
    
    Ok(())
}

fn parse_u32(data: &[u8], offset: usize) -> u32 {
    if offset + 4 > data.len() {
        return 0;
    }
    u32::from_be_bytes([data[offset], data[offset+1], data[offset+2], data[offset+3]])
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

fn build_ssh_packet(payload: &[u8]) -> Vec<u8> {
    let mut packet = Vec::new();
    packet.extend_from_slice(&((payload.len() + 1) as u32).to_be_bytes());
    packet.extend_from_slice(payload);
    packet
}

fn build_ssh_msg_userauth_banner(msg: &str) -> Vec<u8> {
    let mut payload = vec![53];
    payload.extend_from_slice(&(msg.len() as u32).to_be_bytes());
    payload.extend_from_slice(msg.as_bytes());
    payload.extend_from_slice(&4u32.to_be_bytes());
    payload.extend_from_slice(b"en-US");
    build_ssh_packet(&payload)
}

fn build_ssh_msg_userauth_success() -> Vec<u8> {
    build_ssh_packet(&[52])
}

fn build_ssh_msg_userauth_failure() -> Vec<u8> {
    let mut payload = vec![51];
    payload.extend_from_slice(&8u32.to_be_bytes());
    payload.extend_from_slice(b"password");
    payload.push(0);
    build_ssh_packet(&payload)
}

fn build_sftp_packet(payload: &[u8]) -> Vec<u8> {
    let mut packet = Vec::new();
    packet.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    packet.extend_from_slice(payload);
    packet
}

fn build_sftp_version_response(version: u32) -> Vec<u8> {
    let mut payload = vec![2];
    payload.extend_from_slice(&version.to_be_bytes());
    build_sftp_packet(&payload)
}

fn build_sftp_handle_response(id: u32, handle: &str) -> Vec<u8> {
    let mut payload = vec![102];
    payload.extend_from_slice(&id.to_be_bytes());
    payload.extend_from_slice(&(handle.len() as u32).to_be_bytes());
    payload.extend_from_slice(handle.as_bytes());
    build_sftp_packet(&payload)
}

fn build_sftp_status_response(id: u32, status: u32, msg: &str, lang: &str) -> Vec<u8> {
    let mut payload = vec![101];
    payload.extend_from_slice(&id.to_be_bytes());
    payload.extend_from_slice(&status.to_be_bytes());
    payload.extend_from_slice(&(msg.len() as u32).to_be_bytes());
    payload.extend_from_slice(msg.as_bytes());
    payload.extend_from_slice(&(lang.len() as u32).to_be_bytes());
    payload.extend_from_slice(lang.as_bytes());
    build_sftp_packet(&payload)
}

fn build_sftp_data_response(id: u32, data: &[u8]) -> Vec<u8> {
    let mut payload = vec![103];
    payload.extend_from_slice(&id.to_be_bytes());
    payload.extend_from_slice(&(data.len() as u32).to_be_bytes());
    payload.extend_from_slice(data);
    build_sftp_packet(&payload)
}

fn build_sftp_name_response(id: u32, names: &[String]) -> Vec<u8> {
    let mut payload = vec![104];
    payload.extend_from_slice(&id.to_be_bytes());
    payload.extend_from_slice(&(names.len() as u32).to_be_bytes());
    
    for name in names {
        payload.extend_from_slice(&(name.len() as u32).to_be_bytes());
        payload.extend_from_slice(name.as_bytes());
        payload.extend_from_slice(&0u32.to_be_bytes());
        payload.extend_from_slice(&0u32.to_be_bytes());
        payload.extend_from_slice(&0u32.to_be_bytes());
        payload.extend_from_slice(&0u32.to_be_bytes());
        payload.extend_from_slice(&0u32.to_be_bytes());
        payload.extend_from_slice(&0u32.to_be_bytes());
        payload.extend_from_slice(&0u32.to_be_bytes());
        payload.extend_from_slice(&0u32.to_be_bytes());
    }
    
    build_sftp_packet(&payload)
}
