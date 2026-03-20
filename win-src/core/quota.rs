use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;
use std::fs;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QuotaUsage {
    pub used_bytes: u64,
    pub last_updated: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QuotaData {
    pub users: HashMap<String, QuotaUsage>,
}

pub struct QuotaManager {
    data_path: PathBuf,
    data: Arc<Mutex<QuotaData>>,
}

impl QuotaManager {
    pub fn new(data_dir: &Path) -> Self {
        let data_path = data_dir.join("quota.json");
        let data = Self::load_data(&data_path).unwrap_or_default();
        
        QuotaManager {
            data_path,
            data: Arc::new(Mutex::new(data)),
        }
    }

    fn load_data(path: &Path) -> Result<QuotaData> {
        if !path.exists() {
            return Ok(QuotaData::default());
        }
        
        let content = fs::read_to_string(path)?;
        if content.trim().is_empty() {
            return Ok(QuotaData::default());
        }
        
        let data: QuotaData = serde_json::from_str(&content)?;
        Ok(data)
    }

    async fn save_data(&self) -> Result<()> {
        let data = self.data.lock().await;
        if let Some(parent) = self.data_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(&*data)?;
        fs::write(&self.data_path, content)?;
        Ok(())
    }

    pub async fn get_usage(&self, username: &str) -> u64 {
        let data = self.data.lock().await;
        data.users
            .get(username)
            .map(|u| u.used_bytes)
            .unwrap_or(0)
    }

    pub async fn check_quota(
        &self,
        username: &str,
        quota_mb: u64,
        additional_bytes: u64,
    ) -> Result<bool> {
        let data = self.data.lock().await;
        let used = data.users
            .get(username)
            .map(|u| u.used_bytes)
            .unwrap_or(0);
        
        let quota_bytes = quota_mb * 1024 * 1024;
        Ok(used.saturating_add(additional_bytes) <= quota_bytes)
    }

    pub async fn add_usage(&self, username: &str, bytes: u64) -> Result<()> {
        {
            let mut data = self.data.lock().await;
            let usage = data.users.entry(username.to_string()).or_default();
            usage.used_bytes = usage.used_bytes.saturating_add(bytes);
            usage.last_updated = chrono::Utc::now();
        }
        self.save_data().await?;
        Ok(())
    }

    pub async fn subtract_usage(&self, username: &str, bytes: u64) -> Result<()> {
        {
            let mut data = self.data.lock().await;
            if let Some(usage) = data.users.get_mut(username) {
                usage.used_bytes = usage.used_bytes.saturating_sub(bytes);
                usage.last_updated = chrono::Utc::now();
            }
        }
        self.save_data().await?;
        Ok(())
    }

    pub async fn reset_usage(&self, username: &str) -> Result<()> {
        {
            let mut data = self.data.lock().await;
            data.users.remove(username);
        }
        self.save_data().await?;
        Ok(())
    }

    pub async fn recalculate_usage(&self, username: &str, home_dir: &Path) -> Result<u64> {
        let total_size = Self::calculate_dir_size(home_dir)?;
        {
            let mut data = self.data.lock().await;
            let usage = data.users.entry(username.to_string()).or_default();
            usage.used_bytes = total_size;
            usage.last_updated = chrono::Utc::now();
        }
        self.save_data().await?;
        Ok(total_size)
    }

    fn calculate_dir_size(path: &Path) -> Result<u64> {
        let mut total_size = 0u64;
        
        if path.is_file() {
            return Ok(fs::metadata(path)?.len());
        }
        
        if path.is_dir() {
            let entries = fs::read_dir(path)?;
            for entry in entries {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() {
                    total_size = total_size.saturating_add(Self::calculate_dir_size(&path)?);
                } else {
                    total_size = total_size.saturating_add(entry.metadata()?.len());
                }
            }
        }
        
        Ok(total_size)
    }

    pub async fn get_all_usage(&self) -> HashMap<String, QuotaUsage> {
        let data = self.data.lock().await;
        data.users.clone()
    }
}

pub fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    
    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}
