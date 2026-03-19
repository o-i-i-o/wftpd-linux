use anyhow::Result;
use std::fs;
use std::path::Path;
use std::process::Command as StdCommand;

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
        
        let ftp_running = Self::check_port_listening(ftp_port);
        let sftp_running = Self::check_port_listening(sftp_port);
        
        Ok(ServerStatus {
            ftp_running,
            sftp_running,
        })
    }
    
    fn load_ports_from_config() -> (u16, u16) {
        let config_path = Path::new("/etc/wftpg/config.toml");
        if let Ok(content) = std::fs::read_to_string(config_path)
            && let Ok(config) = toml::from_str::<crate::core::config::Config>(&content) {
                return (config.server.ftp_port, config.server.sftp_port);
            }
        (2121, 2222)
    }
    
    fn check_port_listening(port: u16) -> bool {
        if Self::check_linux_proc_net_tcp(port) {
            return true;
        }
        Self::check_port_connectivity(port)
    }
    
    fn check_linux_proc_net_tcp(port: u16) -> bool {
        let port_hex = format!("{:04X}", port);
        
        if let Ok(content) = fs::read_to_string("/proc/net/tcp") {
            for line in content.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 4 {
                    let state = parts[3];
                    if state == "0A" {
                        let local_addr = parts[1];
                        if local_addr.ends_with(&port_hex) {
                            return true;
                        }
                    }
                }
            }
        }
        
        if let Ok(content) = fs::read_to_string("/proc/net/tcp6") {
            for line in content.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 4 {
                    let state = parts[3];
                    if state == "0A" {
                        let local_addr = parts[1];
                        if local_addr.ends_with(&port_hex) {
                            return true;
                        }
                    }
                }
            }
        }
        
        false
    }
    
    fn check_port_connectivity(port: u16) -> bool {
        use std::net::{TcpStream, SocketAddr};
        use std::time::Duration;
        
        let addr: SocketAddr = match format!("127.0.0.1:{}", port).parse() {
            Ok(a) => a,
            Err(_) => return false,
        };
        
        TcpStream::connect_timeout(&addr, Duration::from_millis(100)).is_ok()
    }
    
    fn run_systemctl(args: &[&str]) -> Result<(bool, String)> {
        let output = StdCommand::new("pkexec")
            .arg("systemctl")
            .args(args)
            .output();
        
        match output {
            Ok(output) => {
                let success = output.status.success();
                let message = if success {
                    String::from_utf8_lossy(&output.stdout).trim().to_string()
                } else {
                    String::from_utf8_lossy(&output.stderr).trim().to_string()
                };
                Ok((success, message))
            }
            Err(e) => {
                Ok((false, format!("执行命令失败: {}", e)))
            }
        }
    }
    
    pub fn start_ftp() -> Result<IpcResponse> {
        let (success, message) = Self::run_systemctl(&["start", "wftpd"])?;
        
        if success {
            std::thread::sleep(std::time::Duration::from_millis(500));
            let (ftp_port, _) = Self::load_ports_from_config();
            if Self::check_port_listening(ftp_port) {
                Ok(IpcResponse {
                    success: true,
                    message: "FTP 服务器启动成功".to_string(),
                })
            } else {
                Ok(IpcResponse {
                    success: false,
                    message: "FTP 服务器启动失败: 端口未监听".to_string(),
                })
            }
        } else {
            Ok(IpcResponse {
                success: false,
                message: format!("FTP 服务器启动失败: {}", message),
            })
        }
    }
    
    pub fn stop_ftp() -> Result<IpcResponse> {
        let (success, message) = Self::run_systemctl(&["stop", "wftpd"])?;
        
        Ok(IpcResponse {
            success,
            message: if success {
                "FTP 服务器停止成功".to_string()
            } else {
                format!("FTP 服务器停止失败: {}", message)
            },
        })
    }
    
    pub fn start_sftp() -> Result<IpcResponse> {
        let (success, message) = Self::run_systemctl(&["start", "wftpd"])?;
        
        if success {
            std::thread::sleep(std::time::Duration::from_millis(500));
            let (_, sftp_port) = Self::load_ports_from_config();
            if Self::check_port_listening(sftp_port) {
                Ok(IpcResponse {
                    success: true,
                    message: "SFTP 服务器启动成功".to_string(),
                })
            } else {
                Ok(IpcResponse {
                    success: false,
                    message: "SFTP 服务器启动失败: 端口未监听".to_string(),
                })
            }
        } else {
            Ok(IpcResponse {
                success: false,
                message: format!("SFTP 服务器启动失败: {}", message),
            })
        }
    }
    
    pub fn stop_sftp() -> Result<IpcResponse> {
        let (success, message) = Self::run_systemctl(&["stop", "wftpd"])?;
        
        Ok(IpcResponse {
            success,
            message: if success {
                "SFTP 服务器停止成功".to_string()
            } else {
                format!("SFTP 服务器停止失败: {}", message)
            },
        })
    }
    
    pub fn restart_service() -> Result<IpcResponse> {
        let (success, message) = Self::run_systemctl(&["restart", "wftpd"])?;
        
        Ok(IpcResponse {
            success,
            message: if success {
                "服务重启成功".to_string()
            } else {
                format!("服务重启失败: {}", message)
            },
        })
    }
    
    pub fn is_service_running() -> bool {
        Self::run_systemctl(&["is-active", "--quiet", "wftpd"])
            .map(|(success, _)| success)
            .unwrap_or(false)
    }
}

pub struct ServerStatus {
    pub ftp_running: bool,
    pub sftp_running: bool,
}
