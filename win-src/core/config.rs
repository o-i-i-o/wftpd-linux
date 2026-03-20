use anyhow::{Result, Context};
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub ftp: FtpConfig,
    pub sftp: SftpConfig,
    pub security: SecurityConfig,
    pub logging: LoggingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub bind_ip: String,
    pub ftp_port: u16,
    pub sftp_port: u16,
    pub max_connections: usize,
    pub connection_timeout: u64,
    pub idle_timeout: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FtpConfig {
    pub enabled: bool,
    #[serde(default = "default_bind_ip")]
    pub bind_ip: String,
    pub passive_ports: (u16, u16),
    pub welcome_message: String,
    pub allow_anonymous: bool,
    #[serde(default)]
    pub anonymous_home: Option<String>,
    #[serde(default)]
    pub max_speed_kbps: u64,
    #[serde(default = "default_encoding")]
    pub encoding: String,
    #[serde(default = "default_transfer_mode")]
    pub default_transfer_mode: String,
    #[serde(default = "default_passive_mode")]
    pub default_passive_mode: bool,
    #[serde(default)]
    pub ftps: FtpsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FtpsConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub require_ssl: bool,
    #[serde(default)]
    pub cert_path: Option<String>,
    #[serde(default)]
    pub key_path: Option<String>,
    #[serde(default)]
    pub implicit_ssl: bool,
    #[serde(default = "default_ftps_port")]
    pub implicit_ssl_port: u16,
}

fn default_ftps_port() -> u16 {
    990
}

fn default_bind_ip() -> String {
    "0.0.0.0".to_string()
}

fn default_encoding() -> String {
    "UTF-8".to_string()
}

fn default_transfer_mode() -> String {
    "binary".to_string()
}

fn default_passive_mode() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SftpConfig {
    pub enabled: bool,
    #[serde(default = "default_bind_ip")]
    pub bind_ip: String,
    pub host_key_path: String,
    pub max_auth_attempts: u32,
    pub auth_timeout: u64,
    #[serde(default = "default_log_level")]
    pub log_level: String,
}

fn default_log_level() -> String {
    "info".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityConfig {
    pub allowed_ips: Vec<String>,
    pub denied_ips: Vec<String>,
    pub max_login_attempts: u32,
    pub ban_duration: u64,
    pub require_ssl: bool,
    #[serde(default)]
    pub cert_path: Option<String>,
    #[serde(default)]
    pub key_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    pub log_dir: String,
    pub log_level: String,
    pub max_log_size: u64,
    pub max_log_files: usize,
    pub log_to_file: bool,
    pub log_to_gui: bool,
}

pub fn get_program_data_path() -> PathBuf {
    let program_data = env::var("PROGRAMDATA").unwrap_or_else(|_| "C:\\ProgramData".to_string());
    PathBuf::from(&program_data).join("wftpg")
}

fn get_default_paths() -> (String, String, String) {
    let base_path = get_program_data_path();
    
    let log_dir = base_path.join("logs").to_string_lossy().to_string();
    let host_key_path = base_path.join("ssh\\ssh_host_rsa_key").to_string_lossy().to_string();
    let anonymous_home = base_path.join("anonymous").to_string_lossy().to_string();
    
    (log_dir, host_key_path, anonymous_home)
}

impl Default for Config {
    fn default() -> Self {
        let (log_dir, host_key_path, anonymous_home) = get_default_paths();
        
        Config {
            server: ServerConfig {
                bind_ip: "0.0.0.0".to_string(),
                ftp_port: 21,
                sftp_port: 2222,
                max_connections: 100,
                connection_timeout: 300,
                idle_timeout: 600,
            },
            ftp: FtpConfig {
                enabled: true,
                bind_ip: "0.0.0.0".to_string(),
                passive_ports: (50000, 50100),
                welcome_message: "Welcome to WFTPG FTP Server".to_string(),
                allow_anonymous: false,
                anonymous_home: Some(anonymous_home),
                max_speed_kbps: 0,
                encoding: "UTF-8".to_string(),
                default_transfer_mode: "binary".to_string(),
                default_passive_mode: true,
                ftps: FtpsConfig {
                    enabled: false,
                    require_ssl: false,
                    cert_path: None,
                    key_path: None,
                    implicit_ssl: false,
                    implicit_ssl_port: 990,
                },
            },
            sftp: SftpConfig {
                enabled: true,
                bind_ip: "0.0.0.0".to_string(),
                host_key_path,
                max_auth_attempts: 3,
                auth_timeout: 60,
                log_level: "info".to_string(),
            },
            security: SecurityConfig {
                allowed_ips: vec!["0.0.0.0/0".to_string()],
                denied_ips: vec![],
                max_login_attempts: 5,
                ban_duration: 300,
                require_ssl: false,
                cert_path: None,
                key_path: None,
            },
            logging: LoggingConfig {
                log_dir,
                log_level: "info".to_string(),
                max_log_size: 10 * 1024 * 1024,
                max_log_files: 10,
                log_to_file: true,
                log_to_gui: true,
            },
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            let config = Self::default();
            if let Err(e) = config.save(path) {
                log::warn!("Failed to save default config: {}", e);
            }
            return Ok(config);
        }
        
        let content = fs::read_to_string(path)
            .context("Failed to read config file")?;
        
        let config: Config = toml::from_str(&content)
            .context("Failed to parse config file")?;
        
        Ok(config)
    }

    pub fn validate_paths(&self) -> Vec<String> {
        let mut warnings = Vec::new();
        
        if self.ftp.allow_anonymous {
            match &self.ftp.anonymous_home {
                None => {
                    warnings.push("匿名用户已启用，但未配置匿名用户主目录".to_string());
                }
                Some(anon_home) => {
                    if let Err(e) = Self::validate_home_path(anon_home, "FTP匿名用户主目录") {
                        warnings.push(e);
                    }
                }
            }
        }
        
        if self.sftp.enabled {
            let host_key_path = Path::new(&self.sftp.host_key_path);
            if !host_key_path.exists() {
                warnings.push(format!("SFTP已启用，但主机密钥文件不存在: {}", self.sftp.host_key_path));
            }
        }
        
        if self.ftp.ftps.enabled {
            if let Some(cert_path) = &self.ftp.ftps.cert_path {
                if !Path::new(cert_path).exists() {
                    warnings.push(format!("FTPS已启用，但证书文件不存在: {}", cert_path));
                }
            } else {
                warnings.push("FTPS已启用，但未配置证书路径".to_string());
            }
            
            if let Some(key_path) = &self.ftp.ftps.key_path {
                if !Path::new(key_path).exists() {
                    warnings.push(format!("FTPS已启用，但私钥文件不存在: {}", key_path));
                }
            } else {
                warnings.push("FTPS已启用，但未配置私钥路径".to_string());
            }
        }
        
        {
            let log_dir = &self.logging.log_dir;
            let log_path = Path::new(log_dir);
            if !log_path.exists() {
                warnings.push(format!("日志目录不存在: {}", log_dir));
            } else if fs::metadata(log_path).map(|m| m.permissions().readonly()).unwrap_or(true) {
                warnings.push(format!("日志目录不可写: {}", log_dir));
            }
        }
        
        warnings
    }

    fn validate_home_path(path: &str, name: &str) -> Result<(), String> {
        let p = Path::new(path);
        if !p.exists() {
            return Err(format!("{}不存在: {}", name, path));
        }
        if !p.is_dir() {
            return Err(format!("{}不是目录: {}", name, path));
        }
        if p.canonicalize().is_err() {
            return Err(format!("{}路径无法规范化: {}", name, path));
        }
        Ok(())
    }
    
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .context("Failed to create config directory")?;
        }
        
        let content = toml::to_string_pretty(self)
            .context("Failed to serialize config")?;
        
        fs::write(path, content)
            .context("Failed to write config file")?;
        
        Ok(())
    }
    
    pub fn get_config_path() -> PathBuf {
        get_program_data_path().join("config.toml")
    }
    
    pub fn get_users_path() -> PathBuf {
        get_program_data_path().join("users.json")
    }
    
    pub fn is_ip_allowed(&self, ip: &str) -> bool {
        if self.security.denied_ips.iter().any(|cidr| {
            ip_matches_cidr(ip, cidr).unwrap_or(false)
        }) {
            return false;
        }
        
        if self.security.allowed_ips.is_empty() {
            return true;
        }
        
        self.security.allowed_ips.iter().any(|cidr| {
            ip_matches_cidr(ip, cidr).unwrap_or(false)
        })
    }
}

fn ip_matches_cidr(ip: &str, cidr: &str) -> Result<bool> {
    use std::net::{Ipv4Addr, Ipv6Addr};
    use ipnet::{Ipv4Net, Ipv6Net};
    
    if cidr == "0.0.0.0/0" || cidr == "::/0" {
        return Ok(true);
    }
    
    if let Ok(ipv4) = ip.parse::<Ipv4Addr>()
        && let Ok(net) = cidr.parse::<Ipv4Net>() {
            return Ok(net.contains(&ipv4));
        }
    
    if let Ok(ipv6) = ip.parse::<Ipv6Addr>()
        && let Ok(net) = cidr.parse::<Ipv6Net>() {
            return Ok(net.contains(&ipv6));
        }
    
    Ok(ip == cidr)
}
