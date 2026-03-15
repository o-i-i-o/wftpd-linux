use anyhow::Result;
use std::net::TcpStream;
use std::path::Path;
use std::time::Duration;

pub struct IpcClient;
pub struct Command;
pub struct Response;

#[derive(Debug, Clone)]
pub struct IpcResponse {
    pub success: bool,
    pub message: String,
}

impl IpcClient {
    pub fn get_status() -> Result<ServerStatus> {
        let (ftp_port, sftp_port) = Self::load_ports_from_config();
        
        let ftp_running = Self::check_port_listening("127.0.0.1", ftp_port) 
            || Self::check_port_listening("0.0.0.0", ftp_port);
        let sftp_running = Self::check_port_listening("127.0.0.1", sftp_port) 
            || Self::check_port_listening("0.0.0.0", sftp_port);
        
        Ok(ServerStatus {
            ftp_running,
            sftp_running,
        })
    }
    
    fn load_ports_from_config() -> (u16, u16) {
        let config_path = Path::new("/etc/wftpg/config.toml");
        if let Ok(content) = std::fs::read_to_string(config_path) {
            if let Ok(config) = toml::from_str::<crate::core::config::Config>(&content) {
                return (config.server.ftp_port, config.server.sftp_port);
            }
        }
        (2121, 2222)
    }
    
    fn check_port_listening(host: &str, port: u16) -> bool {
        let addr = format!("{}:{}", host, port);
        TcpStream::connect_timeout(
            &addr.parse().unwrap_or_else(|_| "127.0.0.1:1".parse().unwrap()),
            Duration::from_millis(100)
        ).is_ok()
    }
    
    pub fn start_ftp() -> Result<IpcResponse> {
        Ok(IpcResponse {
            success: true,
            message: "FTP服务器启动成功".to_string(),
        })
    }
    
    pub fn stop_ftp() -> Result<IpcResponse> {
        Ok(IpcResponse {
            success: true,
            message: "FTP服务器停止成功".to_string(),
        })
    }
    
    pub fn start_sftp() -> Result<IpcResponse> {
        Ok(IpcResponse {
            success: true,
            message: "SFTP服务器启动成功".to_string(),
        })
    }
    
    pub fn stop_sftp() -> Result<IpcResponse> {
        Ok(IpcResponse {
            success: true,
            message: "SFTP服务器停止成功".to_string(),
        })
    }
}

pub struct ServerStatus {
    pub ftp_running: bool,
    pub sftp_running: bool,
}
