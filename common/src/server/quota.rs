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

#[cfg(test)]
mod tests {
    use super::*;

    fn block_on<F: std::future::Future>(fut: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(fut)
    }

    fn write_file(path: &Path, bytes: usize) {
        std::fs::write(path, vec![0u8; bytes]).unwrap();
    }

    #[test]
    fn usage_sums_files_recursively() {
        let dir = tempfile::tempdir().unwrap();
        write_file(&dir.path().join("a.txt"), 100);
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        write_file(&dir.path().join("sub/b.txt"), 50);

        let cache = QuotaCache::new();
        let usage = block_on(cache.calculate_usage_async(&dir.path().to_string_lossy()));
        assert_eq!(usage, 150);
    }

    #[test]
    fn usage_of_single_file_is_its_size() {
        let dir = tempfile::tempdir().unwrap();
        write_file(&dir.path().join("only.txt"), 42);
        let cache = QuotaCache::new();
        assert_eq!(
            block_on(cache.calculate_usage_async(&dir.path().join("only.txt").to_string_lossy())),
            42
        );
    }

    #[test]
    fn usage_of_missing_path_is_zero() {
        let cache = QuotaCache::new();
        assert_eq!(
            block_on(cache.calculate_usage_async("/nonexistent/wftpd/dir")),
            0
        );
    }

    #[test]
    fn empty_dir_has_zero_usage() {
        let dir = tempfile::tempdir().unwrap();
        let cache = QuotaCache::new();
        assert_eq!(
            block_on(cache.calculate_usage_async(&dir.path().to_string_lossy())),
            0
        );
    }

    #[test]
    fn usage_tracks_file_growth_and_invalidate_is_harmless() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().to_string_lossy().into_owned();
        let cache = QuotaCache::new();

        write_file(&dir.path().join("x.txt"), 10);
        assert_eq!(block_on(cache.calculate_usage_async(&home)), 10);

        write_file(&dir.path().join("y.txt"), 20);
        // 当前实现每次调用都重新遍历目录（缓存仅作记录），
        // 因此新增文件立即反映在用量中
        assert_eq!(block_on(cache.calculate_usage_async(&home)), 30);
        block_on(cache.invalidate(&home));
        assert_eq!(block_on(cache.calculate_usage_async(&home)), 30);
    }
}
