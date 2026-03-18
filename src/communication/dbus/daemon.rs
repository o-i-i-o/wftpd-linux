use zbus::connection::Builder;
use zbus::Connection;
use std::ffi::CString;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;
use chrono::Utc;

const CONFIG_PATH: &str = "/etc/wftpg/config.toml";
const USERS_PATH: &str = "/etc/wftpg/users.json";
const AUDIT_LOG_PATH: &str = "/var/log/wftpg/audit.log";
const WFTPG_GROUP_NAME: &str = "wftpg";

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
    
    let user = nix::unistd::User::from_uid(nix::unistd::Uid::from_raw(uid))
        .map_err(|e| zbus::fdo::Error::Failed(format!("获取用户信息失败: {}", e)))?
        .ok_or_else(|| zbus::fdo::Error::Failed("用户不存在".to_string()))?;
    
    let wftpg_group = nix::unistd::Group::from_name(WFTPG_GROUP_NAME)
        .map_err(|e| zbus::fdo::Error::Failed(format!("获取 wftpg 组信息失败: {}", e)))?;
    
    if let Some(group) = wftpg_group {
        let user_name_c = CString::new(user.name.as_bytes())
            .map_err(|e| zbus::fdo::Error::Failed(format!("用户名转换失败: {}", e)))?;
        let user_groups = nix::unistd::getgrouplist(&user_name_c, user.gid)
            .map_err(|e| zbus::fdo::Error::Failed(format!("获取用户组列表失败: {}", e)))?;
        
        for g in user_groups {
            if g == group.gid {
                return Ok(());
            }
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
        let creds = conn.peer_creds().await
            .map_err(|e| zbus::fdo::Error::Failed(format!("获取调用者凭证失败: {}", e)))?;
        let uid = creds.unix_user_id()
            .ok_or_else(|| zbus::fdo::Error::Failed("无法获取调用者 UID".to_string()))?;
        check_permission(uid)?;
        let path = self.config_path.lock().await.clone();
        fs::read_to_string(&path)
            .map_err(|e| zbus::fdo::Error::Failed(format!("读取配置失败: {}", e)))
    }
    
    async fn write_config(&self, content: &str, #[zbus(connection)] conn: &Connection) -> zbus::fdo::Result<()> {
        let creds = conn.peer_creds().await
            .map_err(|e| zbus::fdo::Error::Failed(format!("获取调用者凭证失败: {}", e)))?;
        let uid = creds.unix_user_id()
            .ok_or_else(|| zbus::fdo::Error::Failed("无法获取调用者 UID".to_string()))?;
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
        let creds = conn.peer_creds().await
            .map_err(|e| zbus::fdo::Error::Failed(format!("获取调用者凭证失败: {}", e)))?;
        let uid = creds.unix_user_id()
            .ok_or_else(|| zbus::fdo::Error::Failed("无法获取调用者 UID".to_string()))?;
        check_permission(uid)?;
        let path = self.users_path.lock().await.clone();
        if !Path::new(&path).exists() {
            return Ok("{}".to_string());
        }
        fs::read_to_string(&path)
            .map_err(|e| zbus::fdo::Error::Failed(format!("读取用户配置失败: {}", e)))
    }
    
    async fn write_users(&self, content: &str, #[zbus(connection)] conn: &Connection) -> zbus::fdo::Result<()> {
        let creds = conn.peer_creds().await
            .map_err(|e| zbus::fdo::Error::Failed(format!("获取调用者凭证失败: {}", e)))?;
        let uid = creds.unix_user_id()
            .ok_or_else(|| zbus::fdo::Error::Failed("无法获取调用者 UID".to_string()))?;
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
        let creds = conn.peer_creds().await
            .map_err(|e| zbus::fdo::Error::Failed(format!("获取调用者凭证失败: {}", e)))?;
        let uid = creds.unix_user_id()
            .ok_or_else(|| zbus::fdo::Error::Failed("无法获取调用者 UID".to_string()))?;
        check_permission(uid)?;
        let path = self.config_path.lock().await.clone();
        Ok(Path::new(&path).exists())
    }
    
    async fn users_exists(&self, #[zbus(connection)] conn: &Connection) -> zbus::fdo::Result<bool> {
        let creds = conn.peer_creds().await
            .map_err(|e| zbus::fdo::Error::Failed(format!("获取调用者凭证失败: {}", e)))?;
        let uid = creds.unix_user_id()
            .ok_or_else(|| zbus::fdo::Error::Failed("无法获取调用者 UID".to_string()))?;
        check_permission(uid)?;
        let path = self.users_path.lock().await.clone();
        Ok(Path::new(&path).exists())
    }
    
    async fn write_audit_log(&self, user: &str, action: &str, target: &str, details: &str, #[zbus(connection)] conn: &Connection) -> zbus::fdo::Result<()> {
        let creds = conn.peer_creds().await
            .map_err(|e| zbus::fdo::Error::Failed(format!("获取调用者凭证失败: {}", e)))?;
        let uid = creds.unix_user_id()
            .ok_or_else(|| zbus::fdo::Error::Failed("无法获取调用者 UID".to_string()))?;
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
            .map_err(|e| zbus::fdo::Error::Failed(format!("序列化审计日志失败: {}", e)))?
            + "\n";
        
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(AUDIT_LOG_PATH)
            .map_err(|e| zbus::fdo::Error::Failed(format!("打开审计日志失败: {}", e)))?;
        
        use std::io::Write;
        file.write_all(log_line.as_bytes())
            .map_err(|e| zbus::fdo::Error::Failed(format!("写入审计日志失败: {}", e)))?;
        file.sync_all()
            .map_err(|e| zbus::fdo::Error::Failed(format!("同步审计日志失败: {}", e)))?;
        
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
    
    let _conn = Builder::system()?
        .name("com.wftpg")?
        .serve_at("/com/wftpg/Config", config)?
        .build()
        .await?;
    
    log::info!("WFTPG D-Bus service ready on com.wftpg");
    
    tokio::signal::ctrl_c().await?;
    
    log::info!("WFTPG D-Bus service shutting down...");
    
    Ok(())
}
