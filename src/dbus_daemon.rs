use zbus::ConnectionBuilder;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;
use chrono::Utc;

const CONFIG_PATH: &str = "/etc/wftpg/config.toml";
const USERS_PATH: &str = "/etc/wftpg/users.json";
const AUDIT_LOG_PATH: &str = "/var/log/wftpg/audit.log";

#[derive(Clone, serde::Serialize)]
struct AuditEntry {
    timestamp: String,
    user: String,
    action: String,
    target: String,
    details: String,
}

struct WftpgConfig {
    config_path: Arc<Mutex<String>>,
    users_path: Arc<Mutex<String>>,
}

impl WftpgConfig {
    fn new() -> Self {
        Self {
            config_path: Arc::new(Mutex::new(CONFIG_PATH.to_string())),
            users_path: Arc::new(Mutex::new(USERS_PATH.to_string())),
        }
    }
}

#[zbus::interface(name = "com.wftpg.Config")]
impl WftpgConfig {
    async fn read_config(&self) -> zbus::fdo::Result<String> {
        let path = self.config_path.lock().await.clone();
        fs::read_to_string(&path)
            .map_err(|e| zbus::fdo::Error::Failed(format!("读取配置失败: {}", e)))
    }
    
    async fn write_config(&self, content: &str) -> zbus::fdo::Result<()> {
        let path = self.config_path.lock().await.clone();
        if let Some(parent) = Path::new(&path).parent() {
            fs::create_dir_all(parent)
                .map_err(|e| zbus::fdo::Error::Failed(format!("创建目录失败: {}", e)))?;
        }
        fs::write(&path, content)
            .map_err(|e| zbus::fdo::Error::Failed(format!("写入配置失败: {}", e)))
    }
    
    async fn read_users(&self) -> zbus::fdo::Result<String> {
        let path = self.users_path.lock().await.clone();
        if !Path::new(&path).exists() {
            return Ok("{}".to_string());
        }
        fs::read_to_string(&path)
            .map_err(|e| zbus::fdo::Error::Failed(format!("读取用户配置失败: {}", e)))
    }
    
    async fn write_users(&self, content: &str) -> zbus::fdo::Result<()> {
        let path = self.users_path.lock().await.clone();
        if let Some(parent) = Path::new(&path).parent() {
            fs::create_dir_all(parent)
                .map_err(|e| zbus::fdo::Error::Failed(format!("创建目录失败: {}", e)))?;
        }
        fs::write(&path, content)
            .map_err(|e| zbus::fdo::Error::Failed(format!("写入用户配置失败: {}", e)))
    }
    
    async fn config_exists(&self) -> zbus::fdo::Result<bool> {
        let path = self.config_path.lock().await.clone();
        Ok(Path::new(&path).exists())
    }
    
    async fn users_exists(&self) -> zbus::fdo::Result<bool> {
        let path = self.users_path.lock().await.clone();
        Ok(Path::new(&path).exists())
    }
    
    async fn write_audit_log(&self, user: &str, action: &str, target: &str, details: &str) -> zbus::fdo::Result<()> {
        let entry = AuditEntry {
            timestamp: Utc::now().to_rfc3339(),
            user: user.to_string(),
            action: action.to_string(),
            target: target.to_string(),
            details: details.to_string(),
        };
        
        if let Some(parent) = Path::new(AUDIT_LOG_PATH).parent() {
            fs::create_dir_all(parent)
                .map_err(|e| zbus::fdo::Error::Failed(format!("创建审计日志目录失败: {}", e)))?;
        }
        
        let log_line = serde_json::to_string(&entry)
            .unwrap_or_default()
            + "\n";
        
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(AUDIT_LOG_PATH)
            .map_err(|e| zbus::fdo::Error::Failed(format!("打开审计日志失败: {}", e)))?;
        
        use std::io::Write;
        file.write_all(log_line.as_bytes())
            .map_err(|e| zbus::fdo::Error::Failed(format!("写入审计日志失败: {}", e)))?;
        
        Ok(())
    }
}

#[tokio::main]
async fn main() -> zbus::Result<()> {
    if let Ok(j) = systemd_journal_logger::JournalLog::new() {
        let j = j.with_syslog_identifier("wftpg-dbus".to_string());
        let _ = j.install();
    }
    
    log::info!("WFTPG D-Bus Configuration Service starting...");
    
    let config = WftpgConfig::new();
    
    let _conn = ConnectionBuilder::system()?
        .name("com.wftpg")?
        .serve_at("/com/wftpg/Config", config)?
        .build()
        .await?;
    
    log::info!("WFTPG D-Bus service ready on com.wftpg");
    
    tokio::signal::ctrl_c().await?;
    
    log::info!("WFTPG D-Bus service shutting down...");
    Ok(())
}
