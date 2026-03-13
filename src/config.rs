use anyhow::{Result, Context};
use serde::{Deserialize, Serialize};
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
    pub default_home: String,
    pub passive_ports: (u16, u16),
    pub welcome_message: String,
    pub allow_anonymous: bool,
    #[serde(default)]
    pub anonymous_home: Option<String>,
    #[serde(default)]
    pub max_speed_kbps: u64,
    #[serde(default = "default_encoding")]
    pub encoding: String,
}

fn default_encoding() -> String {
    "UTF-8".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SftpConfig {
    pub enabled: bool,
    pub default_home: String,
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

impl Default for Config {
    fn default() -> Self {
        Config {
            server: ServerConfig {
                bind_ip: "0.0.0.0".to_string(),
                ftp_port: 21,
                sftp_port: 22,
                max_connections: 100,
                connection_timeout: 300,
                idle_timeout: 600,
            },
            ftp: FtpConfig {
                enabled: true,
                default_home: "/var/lib/wftpg/share".to_string(),
                passive_ports: (50000, 51000),
                welcome_message: "Welcome to WFTPG FTP Server".to_string(),
                allow_anonymous: false,
                anonymous_home: None,
                max_speed_kbps: 0,
                encoding: "UTF-8".to_string(),
            },
            sftp: SftpConfig {
                enabled: true,
                default_home: "/var/lib/wftpg/share".to_string(),
                host_key_path: "/var/lib/wftpg/ssh/ssh_host_rsa_key".to_string(),
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
                log_dir: "/var/log/wftpg".to_string(),
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
                eprintln!("Warning: Failed to save default config: {}", e);
            }
            return Ok(config);
        }
        
        let content = fs::read_to_string(path)
            .context("Failed to read config file")?;
        
        let config: Config = toml::from_str(&content)
            .context("Failed to parse config file")?;
        
        Ok(config)
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
        PathBuf::from("/etc/wftpg/config.toml")
    }
    
    pub fn get_users_path() -> PathBuf {
        PathBuf::from("/etc/wftpg/users.json")
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
    
    if let Ok(ipv4) = ip.parse::<Ipv4Addr>() {
        if let Ok(net) = cidr.parse::<Ipv4Net>() {
            return Ok(net.contains(&ipv4));
        }
    }
    
    if let Ok(ipv6) = ip.parse::<Ipv6Addr>() {
        if let Ok(net) = cidr.parse::<Ipv6Net>() {
            return Ok(net.contains(&ipv6));
        }
    }
    
    Ok(ip == cidr)
}
