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
After=network.target

[Service]
Type=simple
ExecStart={}
Restart=on-failure
User=wftpg
Group=wftpg
AmbientCapabilities=CAP_NET_BIND_SERVICE

[Install]
WantedBy=multi-user.target
"#,
            binary_path
        );

        fs::write(&self.service_path, service)?;
        Ok(())
    }

    pub fn uninstall_service(&self) -> Result<()> {
        if Path::new(&self.service_path).exists() {
            fs::remove_file(&self.service_path)?;
        }
        Ok(())
    }

    pub fn start_service(&self) -> Result<()> {
        let status = std::process::Command::new("systemctl")
            .args(["start", &self.service_name])
            .status()?;
        
        if !status.success() {
            anyhow::bail!("Failed to start service");
        }
        Ok(())
    }

    pub fn stop_service(&self) -> Result<()> {
        let status = std::process::Command::new("systemctl")
            .args(["stop", &self.service_name])
            .status()?;
        
        if !status.success() {
            anyhow::bail!("Failed to stop service");
        }
        Ok(())
    }

    pub fn restart_service(&self) -> Result<()> {
        let status = std::process::Command::new("systemctl")
            .args(["restart", &self.service_name])
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
        let status = std::process::Command::new("systemctl")
            .arg("daemon-reload")
            .status()?;
        
        if !status.success() {
            anyhow::bail!("Failed to reload daemon");
        }
        Ok(())
    }
}

impl Default for ServiceManager {
    fn default() -> Self {
        Self::new()
    }
}
