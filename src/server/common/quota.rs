use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;
use std::collections::HashMap;
use tracing::{debug, warn};

/// 🔒 配额守卫 - RAII 模式确保配额正确归还
pub struct QuotaGuard {
    bytes: u64,
    home_dir: String,
    quota_cache: Arc<Mutex<HashMap<String, u64>>>,
    committed: bool, // 是否已实际写入
}

impl QuotaGuard {
    /// 创建新的配额守卫
    fn new(bytes: u64, home_dir: String, quota_cache: Arc<Mutex<HashMap<String, u64>>>) -> Self {
        Self {
            bytes,
            home_dir,
            quota_cache,
            committed: false,
        }
    }
    
    /// 标记为已提交（实际写入了数据）
    pub fn commit(&mut self) {
        self.committed = true;
    }
    
    /// 获取已分配的字节数
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
}

impl Drop for QuotaGuard {
    fn drop(&mut self) {
        // 如果没有提交（写入失败或部分写入），归还配额
        if !self.committed {
            debug!("QuotaGuard: Returning {} bytes for {} (not committed)", self.bytes, self.home_dir);
            let mut cache = self.quota_cache.blocking_lock();
            if let Some(usage) = cache.get_mut(&self.home_dir) {
                *usage = usage.saturating_sub(self.bytes);
            }
        } else {
            // 如果提交了，但实际使用少于分配，也需要调整
            // 这将在调用处显式处理
        }
    }
}

pub struct QuotaCache {
    cache: Arc<Mutex<HashMap<String, u64>>>,
}

impl QuotaCache {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn get_usage(&self, home_dir: &str) -> u64 {
        let cache = self.cache.blocking_lock();
        if let Some(&usage) = cache.get(home_dir) {
            return usage;
        }
        drop(cache);
        self.calculate_usage(home_dir)
    }

    pub fn calculate_usage(&self, home_dir: &str) -> u64 {
        let path = Path::new(home_dir);
        let usage = Self::dir_size(path);
        
        let mut cache = self.cache.blocking_lock();
        cache.insert(home_dir.to_string(), usage);
        
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
        let mut cache = self.cache.blocking_lock();
        cache.remove(home_dir);
    }

    /// 🔒 原子性地尝试分配配额
    /// 
    /// 返回 QuotaGuard，如果操作失败或未完成，会自动归还配额
    pub async fn try_allocate(&self, home_dir: &str, quota_mb: u64, additional_bytes: u64) -> Result<QuotaGuard, &'static str> {
        // 配额为 0 表示无限制
        if quota_mb == 0 {
            return Ok(QuotaGuard::new(additional_bytes, home_dir.to_string(), self.cache.clone()));
        }
        
        let quota_bytes = quota_mb * 1024 * 1024;
        
        // 🔒 原子性检查和分配
        let mut cache = self.cache.lock().await;
        let current_usage = cache.get(home_dir).copied().unwrap_or(0);
        
        if current_usage.saturating_add(additional_bytes) > quota_bytes {
            warn!(
                "Quota exceeded for {}: current={}, requested={}, limit={}",
                home_dir, current_usage, additional_bytes, quota_bytes
            );
            return Err("Quota exceeded");
        }
        
        // 分配配额
        *cache.entry(home_dir.to_string()).or_insert(0) += additional_bytes;
        debug!(
            "Quota allocated: {} += {} (now {}), limit={}",
            home_dir, additional_bytes, current_usage + additional_bytes, quota_bytes
        );
        
        Ok(QuotaGuard::new(additional_bytes, home_dir.to_string(), self.cache.clone()))
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
