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
            service_name: "wftpg".to_string(),
            service_path: "/etc/systemd/system/wftpg.service".to_string(),
        }
    }
    
    pub fn install_service(&self, binary_path: &str) -> Result<()> {
        let service_content = format!(
r#"[Unit]
Description=WFTPG - SFTP/FTP Server Management Tool
After=network.target

[Service]
Type=simple
ExecStart={} --service
Restart=on-failure
RestartSec=5
User=root
WorkingDirectory=/etc/wftpg

[Install]
WantedBy=multi-user.target
"#, binary_path);
        
        fs::write(&self.service_path, service_content)?;
        
        Ok(())
    }
    
    pub fn uninstall_service(&self) -> Result<()> {
        if Path::new(&self.service_path).exists() {
            fs::remove_file(&self.service_path)?;
        }
        Ok(())
    }
    
    pub fn start_service(&self) -> Result<()> {
        std::process::Command::new("systemctl")
            .args(&["start", &self.service_name])
            .status()?;
        Ok(())
    }
    
    pub fn stop_service(&self) -> Result<()> {
        std::process::Command::new("systemctl")
            .args(&["stop", &self.service_name])
            .status()?;
        Ok(())
    }
    
    pub fn restart_service(&self) -> Result<()> {
        std::process::Command::new("systemctl")
            .args(&["restart", &self.service_name])
            .status()?;
        Ok(())
    }
    
    pub fn enable_service(&self) -> Result<()> {
        std::process::Command::new("systemctl")
            .args(&["enable", &self.service_name])
            .status()?;
        Ok(())
    }
    
    pub fn disable_service(&self) -> Result<()> {
        std::process::Command::new("systemctl")
            .args(&["disable", &self.service_name])
            .status()?;
        Ok(())
    }
    
    pub fn is_service_running(&self) -> bool {
        std::process::Command::new("systemctl")
            .args(&["is-active", "--quiet", &self.service_name])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    
    pub fn is_service_enabled(&self) -> bool {
        std::process::Command::new("systemctl")
            .args(&["is-enabled", "--quiet", &self.service_name])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    
    pub fn get_service_status(&self) -> String {
        let output = std::process::Command::new("systemctl")
            .args(&["status", &self.service_name])
            .output();
        
        match output {
            Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
            Err(_) => "Unable to get service status".to_string(),
        }
    }
    
    pub fn service_exists(&self) -> bool {
        Path::new(&self.service_path).exists()
    }
    
    pub fn reload_daemon(&self) -> Result<()> {
        std::process::Command::new("systemctl")
            .arg("daemon-reload")
            .status()?;
        Ok(())
    }
}

impl Default for ServiceManager {
    fn default() -> Self {
        Self::new()
    }
}
