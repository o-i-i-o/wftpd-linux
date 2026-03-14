use zbus::ConnectionBuilder;
use zbus::Connection;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;
use chrono::Utc;

const CONFIG_PATH: &str = "/etc/wftpg/config.toml";
const USERS_PATH: &str = "/etc/wftpg/users.json";
const AUDIT_LOG_PATH: &str = "/var/log/wftpg/audit.log";
const WFTPG_GROUP_ID: u32 = 975;

#[derive(Clone, serde::Serialize)]
struct AuditEntry {
    timestamp: String,
    user: String,
    action: String,
    target: String,
    details: String,
}

pub struct WftpgConfig {
    config_path: Arc<Mutex<String>>,
    users_path: Arc<Mutex<String>>,
}

fn check_permission(uid: u32) -> Result<(), zbus::fdo::Error> {
    if uid == 0 {
        return Ok(());
    }
    
    let groups = nix::unistd::getgroups()
        .map_err(|e| zbus::fdo::Error::Failed(format!("获取组列表失败: {}", e)))?;
    
    for group in groups {
        if group.as_raw() == WFTPG_GROUP_ID {
            return Ok(());
        }
    }
    
    Err(zbus::fdo::Error::AccessDenied("权限不足: 需要 root 或 wftpg 组权限".to_string()))
}

impl WftpgConfig {
    pub fn new() -> Self {
        Self {
            config_path: Arc::new(Mutex::new(CONFIG_PATH.to_string())),
            users_path: Arc::new(Mutex::new(USERS_PATH.to_string())),
        }
    }
}

impl Default for WftpgConfig {
    fn default() -> Self {
        Self::new()
    }
}

#[zbus::interface(name = "com.wftpg.Config")]
impl WftpgConfig {
    async fn read_config(&self, #[zbus(connection)] conn: &Connection) -> zbus::fdo::Result<String> {
        let creds = conn.peer_credentials().await
            .map_err(|e| zbus::fdo::Error::Failed(format!("获取调用者凭证失败: {}", e)))?;
        let uid = creds.unix_user_id().unwrap_or(u32::MAX);
        check_permission(uid)?;
        let path = self.config_path.lock().await.clone();
        fs::read_to_string(&path)
            .map_err(|e| zbus::fdo::Error::Failed(format!("读取配置失败: {}", e)))
    }
    
    async fn write_config(&self, content: &str, #[zbus(connection)] conn: &Connection) -> zbus::fdo::Result<()> {
        let creds = conn.peer_credentials().await
            .map_err(|e| zbus::fdo::Error::Failed(format!("获取调用者凭证失败: {}", e)))?;
        let uid = creds.unix_user_id().unwrap_or(u32::MAX);
        check_permission(uid)?;
        let path = self.config_path.lock().await.clone();
        if let Some(parent) = Path::new(&path).parent() {
            fs::create_dir_all(parent)
                .map_err(|e| zbus::fdo::Error::Failed(format!("创建目录失败: {}", e)))?;
        }
        fs::write(&path, content)
            .map_err(|e| zbus::fdo::Error::Failed(format!("写入配置失败: {}", e)))
    }
    
    async fn read_users(&self, #[zbus(connection)] conn: &Connection) -> zbus::fdo::Result<String> {
        let creds = conn.peer_credentials().await
            .map_err(|e| zbus::fdo::Error::Failed(format!("获取调用者凭证失败: {}", e)))?;
        let uid = creds.unix_user_id().unwrap_or(u32::MAX);
        check_permission(uid)?;
        let path = self.users_path.lock().await.clone();
        if !Path::new(&path).exists() {
            return Ok("{}".to_string());
        }
        fs::read_to_string(&path)
            .map_err(|e| zbus::fdo::Error::Failed(format!("读取用户配置失败: {}", e)))
    }
    
    async fn write_users(&self, content: &str, #[zbus(connection)] conn: &Connection) -> zbus::fdo::Result<()> {
        let creds = conn.peer_credentials().await
            .map_err(|e| zbus::fdo::Error::Failed(format!("获取调用者凭证失败: {}", e)))?;
        let uid = creds.unix_user_id().unwrap_or(u32::MAX);
        check_permission(uid)?;
        let path = self.users_path.lock().await.clone();
        if let Some(parent) = Path::new(&path).parent() {
            fs::create_dir_all(parent)
                .map_err(|e| zbus::fdo::Error::Failed(format!("创建目录失败: {}", e)))?;
        }
        fs::write(&path, content)
            .map_err(|e| zbus::fdo::Error::Failed(format!("写入用户配置失败: {}", e)))
    }
    
    async fn config_exists(&self, #[zbus(connection)] conn: &Connection) -> zbus::fdo::Result<bool> {
        let creds = conn.peer_credentials().await
            .map_err(|e| zbus::fdo::Error::Failed(format!("获取调用者凭证失败: {}", e)))?;
        let uid = creds.unix_user_id().unwrap_or(u32::MAX);
        check_permission(uid)?;
        let path = self.config_path.lock().await.clone();
        Ok(Path::new(&path).exists())
    }
    
    async fn users_exists(&self, #[zbus(connection)] conn: &Connection) -> zbus::fdo::Result<bool> {
        let creds = conn.peer_credentials().await
            .map_err(|e| zbus::fdo::Error::Failed(format!("获取调用者凭证失败: {}", e)))?;
        let uid = creds.unix_user_id().unwrap_or(u32::MAX);
        check_permission(uid)?;
        let path = self.users_path.lock().await.clone();
        Ok(Path::new(&path).exists())
    }
    
    async fn write_audit_log(&self, user: &str, action: &str, target: &str, details: &str, #[zbus(connection)] conn: &Connection) -> zbus::fdo::Result<()> {
        let creds = conn.peer_credentials().await
            .map_err(|e| zbus::fdo::Error::Failed(format!("获取调用者凭证失败: {}", e)))?;
        let uid = creds.unix_user_id().unwrap_or(u32::MAX);
        check_permission(uid)?;
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

pub async fn run_daemon() -> zbus::Result<()> {
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
