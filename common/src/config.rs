use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::warn;

use crate::paths;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub ftp: FtpConfig,
    pub sftp: SftpConfig,
    pub security: SecurityConfig,
    pub logging: LoggingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FtpConfig {
    pub enabled: bool,
    #[serde(default = "default_bind_ip")]
    pub bind_ip: String,
    #[serde(default = "default_ftp_port")]
    pub port: u16,
    pub passive_ports: (u16, u16),
    pub welcome_message: String,
    pub allow_anonymous: bool,
    #[serde(default)]
    pub anonymous_home: Option<String>,
    #[serde(default)]
    pub max_speed_kbps: u64,
    #[serde(default = "default_encoding")]
    pub encoding: String,
    #[serde(default = "default_data_timeout")]
    pub data_timeout: u64,
    #[serde(default)]
    pub masquerade_ip: Option<String>,
}

fn default_bind_ip() -> String {
    "0.0.0.0".to_string()
}

fn default_ftp_port() -> u16 {
    21
}

fn default_encoding() -> String {
    "UTF-8".to_string()
}

fn default_data_timeout() -> u64 {
    300
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SftpConfig {
    pub enabled: bool,
    #[serde(default = "default_bind_ip")]
    pub bind_ip: String,
    #[serde(default = "default_sftp_port")]
    pub port: u16,
    pub host_key_path: String,
    pub max_auth_attempts: u32,
    pub auth_timeout: u64,
    #[serde(default = "default_log_level")]
    pub log_level: String,
}

fn default_sftp_port() -> u16 {
    22
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
    #[serde(default = "default_max_connections")]
    pub max_connections: usize,
    #[serde(default = "default_connection_timeout")]
    pub connection_timeout: u64,
    #[serde(default = "default_idle_timeout")]
    pub idle_timeout: u64,
}

fn default_max_connections() -> usize {
    100
}

fn default_connection_timeout() -> u64 {
    300
}

fn default_idle_timeout() -> u64 {
    600
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    pub log_dir: String,
    pub log_level: String,
    pub max_log_size: u64,
    pub max_log_files: usize,
    #[serde(default)]
    pub enable_json: bool,
    /// 允许后端把日志实时推送给前端（WatchLogs 订阅）
    #[serde(default = "default_enable_gui_logging")]
    pub enable_gui_logging: bool,
}

fn default_enable_gui_logging() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Config {
            ftp: FtpConfig {
                enabled: true,
                bind_ip: "0.0.0.0".to_string(),
                port: 2121,
                passive_ports: (50000, 51000),
                welcome_message: "Welcome to WFTPG FTP Server".to_string(),
                allow_anonymous: false,
                anonymous_home: None,
                max_speed_kbps: 0,
                encoding: "UTF-8".to_string(),
                data_timeout: 300,
                masquerade_ip: None,
            },
            sftp: SftpConfig {
                enabled: true,
                bind_ip: "0.0.0.0".to_string(),
                port: 2222,
                host_key_path: paths::default_host_key_path()
                    .to_string_lossy()
                    .into_owned(),
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
                max_connections: 100,
                connection_timeout: 300,
                idle_timeout: 600,
            },
            logging: LoggingConfig {
                log_dir: paths::default_log_dir().to_string_lossy().into_owned(),
                log_level: "info".to_string(),
                max_log_size: 10 * 1024 * 1024,
                max_log_files: 10,
                enable_json: false,
                enable_gui_logging: true,
            },
        }
    }
}

impl Config {
    /// 加载 TOML 配置；文件不存在时落盘一份默认配置并返回默认值
    ///
    /// # Errors
    /// 配置文件存在但读取失败，或内容不是合法的 TOML 配置时返回错误
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            let config = Self::default();
            if let Err(e) = config.save(path) {
                eprintln!("Warning: Failed to save default config: {e}");
            }
            return Ok(config);
        }

        let content = fs::read_to_string(path).context("Failed to read config file")?;

        let config: Config = toml::from_str(&content).context("Failed to parse config file")?;

        Ok(config)
    }

    /// 将配置序列化为 TOML 并写入 `path`（先写临时文件再原子重命名）
    ///
    /// # Errors
    /// 父目录创建、序列化、临时文件写入或重命名失败时返回错误
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("Failed to create config directory")?;
        }

        let content = toml::to_string_pretty(self).context("Failed to serialize config")?;

        let temp_path = path.with_extension("tmp");
        fs::write(&temp_path, content).context("Failed to write temp config file")?;

        if let Err(e) = fs::rename(&temp_path, path) {
            let _ = fs::remove_file(&temp_path);
            return Err(e).context("Failed to rename temp config file");
        }

        Ok(())
    }

    #[must_use]
    pub fn get_config_path() -> PathBuf {
        paths::config_path()
    }

    #[must_use]
    pub fn get_users_path() -> PathBuf {
        paths::users_path()
    }

    /// 校验配置自身的一致性（匿名 FTP 主目录、SFTP 主机密钥）
    ///
    /// # Errors
    /// FTP 匿名访问已启用但 `anonymous_home` 未配置、不存在或不是目录时返回错误
    pub fn validate(&self) -> Result<()> {
        if self.ftp.enabled && self.ftp.allow_anonymous {
            match &self.ftp.anonymous_home {
                Some(home) if !home.trim().is_empty() => {
                    let home_path = Path::new(home);
                    if !home_path.exists() {
                        return Err(anyhow::anyhow!(
                            "FTP匿名访问已启用，但匿名用户主目录不存在: {home}"
                        ));
                    }
                    if !home_path.is_dir() {
                        return Err(anyhow::anyhow!(
                            "FTP匿名访问已启用，但匿名用户主目录路径不是目录: {home}"
                        ));
                    }
                }
                _ => {
                    return Err(anyhow::anyhow!(
                        "FTP匿名访问已启用，但未配置匿名用户主目录(anonymous_home)"
                    ));
                }
            }
        }

        if self.sftp.enabled {
            let host_key = Path::new(&self.sftp.host_key_path);
            if !host_key.exists() {
                warn!(
                    "SFTP主机密钥不存在: {}，请运行安装脚本生成",
                    self.sftp.host_key_path
                );
            }
        }

        Ok(())
    }

    /// 静态版本：供非 Config 持有方（如 FTP 认证桥）复用同一套规则
    #[must_use]
    pub fn is_ip_allowed_for(allowed_ips: &[String], denied_ips: &[String], ip: &str) -> bool {
        if denied_ips.iter().any(|cidr| ip_matches_cidr(ip, cidr)) {
            return false;
        }

        if allowed_ips.is_empty() {
            return true;
        }

        allowed_ips.iter().any(|cidr| ip_matches_cidr(ip, cidr))
    }

    #[must_use]
    pub fn is_ip_allowed(&self, ip: &str) -> bool {
        if self
            .security
            .denied_ips
            .iter()
            .any(|cidr| ip_matches_cidr(ip, cidr))
        {
            return false;
        }

        if self.security.allowed_ips.is_empty() {
            return true;
        }

        self.security
            .allowed_ips
            .iter()
            .any(|cidr| ip_matches_cidr(ip, cidr))
    }
}

fn ip_matches_cidr(ip: &str, cidr: &str) -> bool {
    use ipnet::{Ipv4Net, Ipv6Net};
    use std::net::{Ipv4Addr, Ipv6Addr};

    if cidr == "0.0.0.0/0" || cidr == "::/0" {
        return true;
    }

    if let Ok(ipv4) = ip.parse::<Ipv4Addr>()
        && let Ok(net) = cidr.parse::<Ipv4Net>()
    {
        return net.contains(&ipv4);
    }

    if let Ok(ipv6) = ip.parse::<Ipv6Addr>()
        && let Ok(net) = cidr.parse::<Ipv6Net>()
    {
        return net.contains(&ipv6);
    }

    ip == cidr
}
