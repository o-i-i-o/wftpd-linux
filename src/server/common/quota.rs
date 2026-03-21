use std::path::Path;
use std::sync::Mutex;
use std::collections::HashMap;

pub struct QuotaCache {
    cache: Mutex<HashMap<String, u64>>,
}

impl QuotaCache {
    pub fn new() -> Self {
        Self {
            cache: Mutex::new(HashMap::new()),
        }
    }

    pub fn get_usage(&self, home_dir: &str) -> u64 {
        if let Ok(cache) = self.cache.lock()
            && let Some(&usage) = cache.get(home_dir)
        {
            return usage;
        }
        self.calculate_usage(home_dir)
    }

    pub fn calculate_usage(&self, home_dir: &str) -> u64 {
        let path = Path::new(home_dir);
        let usage = Self::dir_size(path);
        
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(home_dir.to_string(), usage);
        }
        
        usage
    }

    fn dir_size(path: &Path) -> u64 {
        let mut size = 0;
        
        if path.is_dir() {
            if let Ok(entries) = std::fs::read_dir(path) {
                for entry in entries.flatten() {
                    let entry_path = entry.path();
                    if entry_path.is_dir() {
                        size += Self::dir_size(&entry_path);
                    } else if entry_path.is_file()
                        && let Ok(metadata) = entry_path.metadata()
                    {
                        size += metadata.len();
                    }
                }
            }
        } else if path.is_file()
            && let Ok(metadata) = path.metadata()
        {
            size = metadata.len();
        }
        
        size
    }

    pub fn invalidate(&self, home_dir: &str) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.remove(home_dir);
        }
    }

    pub fn check_quota(&self, home_dir: &str, quota_mb: u64, additional_bytes: u64) -> bool {
        if quota_mb == 0 {
            return true;
        }
        
        let quota_bytes = quota_mb * 1024 * 1024;
        let current_usage = self.get_usage(home_dir);
        
        current_usage.saturating_add(additional_bytes) <= quota_bytes
    }
}

impl Default for QuotaCache {
    fn default() -> Self {
        Self::new()
    }
}
