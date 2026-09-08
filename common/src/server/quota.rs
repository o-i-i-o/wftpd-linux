use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct QuotaCache {
    cache: Arc<Mutex<HashMap<String, u64>>>,
}

impl QuotaCache {
    #[must_use]
    pub fn new() -> Self {
        Self {
            cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn calculate_usage_async(&self, home_dir: &str) -> u64 {
        // dir_size 同步遍历目录树，放入阻塞线程池避免卡住异步执行器
        let path = PathBuf::from(home_dir);
        let usage = tokio::task::spawn_blocking(move || Self::dir_size(&path))
            .await
            .unwrap_or(0);

        let mut cache = self.cache.lock().await;
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

    pub async fn invalidate(&self, home_dir: &str) {
        let mut cache = self.cache.lock().await;
        cache.remove(home_dir);
    }
}

impl Default for QuotaCache {
    fn default() -> Self {
        Self::new()
    }
}
