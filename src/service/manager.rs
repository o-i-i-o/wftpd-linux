use anyhow::Result;
use std::fs;
use std::path::Path;

pub struct ServiceManager {
    service_name: String,
    service_path: String,
}

impl ServiceManager {
    pub fn new() -> Self {
        ServiceManager {
            service_name: "wftpd".to_string(),
            service_path: "/lib/systemd/system/wftpd.service".to_string(),
        }
    }

    pub fn install_service(&self, binary_path: &str) -> Result<()> {
        let service = format!(
            r#"[Unit]
Description=WFTPG SFTP/FTP Server
Documentation=https://github.com/wftpg/wftpg
After=network.target network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart={}
Restart=on-failure
RestartSec=5

User=wftpg
Group=wftpg

WorkingDirectory=/var/lib/wftpg

RuntimeDirectory=wftpd
RuntimeDirectoryMode=0770

Environment=HOME=/var/lib/wftpg
Environment=XDG_CONFIG_HOME=/var/lib/wftpg/config
Environment=XDG_CACHE_HOME=/var/lib/wftpg/cache

AmbientCapabilities=CAP_NET_BIND_SERVICE
CapabilityBoundingSet=CAP_NET_BIND_SERVICE

NoNewPrivileges=true

ProtectSystem=strict
ProtectHome=yes
PrivateTmp=yes
PrivateDevices=yes

ReadWritePaths=/var/log/wftpg /var/lib/wftpg /etc/wftpg /run/wftpd

LimitNOFILE=65536

[Install]
WantedBy=multi-user.target
"#,
            binary_path
        );

        fs::write(&self.service_path, service)?;
        
        self.setup_wftpg_user_permissions()?;
        
        Ok(())
    }

    fn setup_wftpg_user_permissions(&self) -> Result<()> {
        if let Some(current_user) = self.get_current_gui_user() {
            let status = std::process::Command::new("usermod")
                .args(["-aG", "wftpg", &current_user])
                .status()?;
            
            if status.success() {
                log::info!("Added {} to wftpg group for service access", current_user);
            } else {
                log::warn!("Failed to add {} to wftpg group, permissions may need manual setup", current_user);
            }
        }
        
        Ok(())
    }

    fn get_current_gui_user(&self) -> Option<String> {
        if let Ok(user) = std::env::var("SUDO_USER") {
            return Some(user);
        }
        
        if let Ok(user) = std::env::var("PKEXEC_UID")
            && let Ok(uid) = user.parse::<u32>() {
                return self.get_username_by_uid(uid);
            }
        
        if let Ok(user) = std::env::var("USER")
            && user != "root" {
                return Some(user);
            }
        
        self.get_session_user()
    }

    fn get_username_by_uid(&self, uid: u32) -> Option<String> {
        use std::ffi::CStr;
        
        unsafe {
            let pwd = libc::getpwuid(uid);
            if !pwd.is_null() {
                let name = CStr::from_ptr((*pwd).pw_name);
                return name.to_str().ok().map(|s| s.to_string());
            }
        }
        None
    }

    fn get_session_user(&self) -> Option<String> {
        let output = std::process::Command::new("loginctl")
            .args(["list-sessions", "--no-legend"])
            .output()
            .ok()?;
        
        let sessions = String::from_utf8_lossy(&output.stdout);
        for line in sessions.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                return Some(parts[2].to_string());
            }
        }
        
        None
    }

    pub fn uninstall_service(&self) -> Result<()> {
        if Path::new(&self.service_path).exists() {
            fs::remove_file(&self.service_path)?;
        }
        Ok(())
    }

    pub fn start_service(&self) -> Result<()> {
        let status = std::process::Command::new("pkexec")
            .args(["systemctl", "start", &self.service_name])
            .status()?;
        
        if !status.success() {
            anyhow::bail!("Failed to start service");
        }
        Ok(())
    }

    pub fn stop_service(&self) -> Result<()> {
        let status = std::process::Command::new("pkexec")
            .args(["systemctl", "stop", &self.service_name])
            .status()?;
        
        if !status.success() {
            anyhow::bail!("Failed to stop service");
        }
        Ok(())
    }

    pub fn restart_service(&self) -> Result<()> {
        let status = std::process::Command::new("pkexec")
            .args(["systemctl", "restart", &self.service_name])
            .status()?;
        
        if !status.success() {
            anyhow::bail!("Failed to restart service");
        }
        Ok(())
    }

    pub fn is_service_running(&self) -> bool {
        std::process::Command::new("systemctl")
            .args(["is-active", "--quiet", &self.service_name])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    pub fn service_exists(&self) -> bool {
        Path::new(&self.service_path).exists()
    }

    pub fn reload_daemon(&self) -> Result<()> {
        let status = std::process::Command::new("pkexec")
            .args(["systemctl", "daemon-reload"])
            .status()?;
        
        if !status.success() {
            anyhow::bail!("Failed to reload daemon");
        }
        Ok(())
    }

    pub fn enable_service(&self) -> Result<()> {
        let status = std::process::Command::new("pkexec")
            .args(["systemctl", "enable", &self.service_name])
            .status()?;
        
        if !status.success() {
            anyhow::bail!("Failed to enable service");
        }
        Ok(())
    }

    pub fn disable_service(&self) -> Result<()> {
        let status = std::process::Command::new("pkexec")
            .args(["systemctl", "disable", &self.service_name])
            .status()?;
        
        if !status.success() {
            anyhow::bail!("Failed to disable service");
        }
        Ok(())
    }

    pub fn is_service_enabled(&self) -> bool {
        std::process::Command::new("systemctl")
            .args(["is-enabled", "--quiet", &self.service_name])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

impl Default for ServiceManager {
    fn default() -> Self {
        Self::new()
    }
}
