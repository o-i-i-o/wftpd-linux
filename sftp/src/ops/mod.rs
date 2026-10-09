//! SFTP 协议操作实现（`russh_sftp::server::Handler`）。
//!
//! 业务逻辑与旧手写实现保持一致：
//! - 路径经词法解析锚定在用户主目录内（防穿越）
//! - 每个操作前检查用户权限（can_read/can_write/...）
//! - 上传写入量受 `quota_mb` 配额约束
//! - 传输受用户 `speed_limit_kbps` 限速
//! - 文件操作写入审计日志（FileLogger → `file_ops` target）
//! - 支持 openssh 扩展：md5sum / sha256sum / space-available
//!
//! 模块划分：
//! - [`protocol`]：SFTP 协议方法（`russh_sftp::server::Handler` trait impl，
//!   Rust 要求同一 trait 的 impl 为单个块，故聚合于此）
//! - [`extended`]：openssh 扩展辅助（md5sum/sha256sum/space-available）
//! - [`tests`]：单元测试

mod extended;
#[cfg(test)]
mod tests;
mod protocol;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};

use russh_sftp::protocol::{FileAttributes, Status, StatusCode};
use wftpd_common::server::quota::QuotaCache;
use wftpd_common::server::speed_limiter::SpeedLimiter;
use wftpd_common::{FileLogger, Permissions, UserManager};

const SFTP_VERSION: u32 = 3;

/// 单次 READ 请求允许的最大字节数（与 OpenSSH 一致取 256 KiB）。
/// len 来自客户端且直接决定缓冲区分配大小，必须设上限防止内存耗尽。
const MAX_READ_LEN: u32 = 256 * 1024;

type SftpResult<T> = Result<T, StatusCode>;

/// 打开的文件句柄
struct FileHandle {
    file: tokio::fs::File,
    writable: bool,
}

/// 打开的目录句柄（持有目录流，readdir 分批枚举）
struct DirHandle {
    rd: tokio::fs::ReadDir,
}

enum OpenEntry {
    File(FileHandle),
    Dir(DirHandle),
}

/// 每个已认证的 SFTP 会话一个实例（由 `russh_sftp::server::run` 驱动）
pub struct SftpFileHandler {
    username: String,
    home_dir: String,
    user_manager: Arc<StdMutex<UserManager>>,
    file_logger: Arc<StdMutex<FileLogger>>,
    quota_cache: Arc<QuotaCache>,
    client_ip: String,
    handles: HashMap<String, OpenEntry>,
    next_handle: u64,
    speed_limiter: Option<SpeedLimiter>,
}

impl SftpFileHandler {
    /// 创建会话处理器；限速参数取自当前用户配置
    ///
    /// # Panics
    /// 用户库互斥锁中毒（持有线程 panic）时 panic
    pub fn new(
        username: String,
        home_dir: String,
        user_manager: Arc<StdMutex<UserManager>>,
        file_logger: Arc<StdMutex<FileLogger>>,
        quota_cache: Arc<QuotaCache>,
        client_ip: String,
    ) -> Self {
        let speed_limit = {
            let users = user_manager.lock().unwrap();
            users
                .get_user(&username)
                .and_then(|u| u.permissions.speed_limit_kbps)
        };
        let speed_limiter = speed_limit.filter(|&kbps| kbps > 0).map(SpeedLimiter::new);

        SftpFileHandler {
            username,
            home_dir,
            user_manager,
            file_logger,
            quota_cache,
            client_ip,
            handles: HashMap::new(),
            next_handle: 1,
            speed_limiter,
        }
    }

    // ---- 内部工具（子模块共享） ----

    fn permissions(&self) -> Permissions {
        let users = self.user_manager.lock().unwrap();
        users
            .get_user(&self.username)
            .map(|u| u.permissions)
            .unwrap_or_default()
    }

    /// 词法解析路径：锚定在用户主目录内，处理 `.`/`..`，不跟随符号链接
    /// （lstat/readlink 等操作要求不跟随；打开文件时的链接跟随由 OS 完成）
    fn resolve(&self, path: &str) -> SftpResult<PathBuf> {
        let home = PathBuf::from(&self.home_dir);
        let stripped = path.trim();
        let base = if stripped.starts_with('/') {
            // 客户端可能直接引用真实绝对路径（主目录前缀），做双重路径防御：
            // 若以 home 开头则视为已在 home 内，否则视为 chroot 内路径
            let rest = stripped.trim_start_matches('/');
            if !rest.is_empty()
                && let Some(within) = PathBuf::from(stripped).strip_prefix(&home).ok()
            {
                return Ok(home.join(within));
            }
            home.join(rest)
        } else {
            home.join(stripped)
        };

        let mut normalized = home.clone();
        for component in base.strip_prefix(&home).unwrap_or(&base).components() {
            match component {
                std::path::Component::Normal(seg) => {
                    normalized.push(seg);
                }
                std::path::Component::CurDir => {}
                std::path::Component::ParentDir => {
                    if normalized == home {
                        return Err(StatusCode::NoSuchFile);
                    }
                    normalized.pop();
                }
                _ => return Err(StatusCode::NoSuchFile),
            }
        }
        Ok(normalized)
    }

    fn audit(
        &self,
        operation: &str,
        file_path: &str,
        file_size: u64,
        success: bool,
        message: &str,
    ) {
        if let Ok(mut file_log) = self.file_logger.try_lock() {
            file_log.log(&wftpd_common::FileLogInfo {
                username: &self.username,
                client_ip: &self.client_ip,
                operation,
                file_path,
                file_size,
                protocol: "SFTP",
                success,
                message,
            });
        }
    }

    fn next_handle_id(&mut self) -> String {
        loop {
            let id = self.next_handle;
            self.next_handle += 1;
            let handle = format!("h{id:x}");
            if !self.handles.contains_key(&handle) {
                return handle;
            }
        }
    }

    async fn quota_ok(&self, additional: u64) -> bool {
        if additional == 0 {
            return true;
        }
        let quota_mb = self.permissions().quota_mb.filter(|&limit| limit > 0);
        let Some(limit) = quota_mb else {
            return true;
        };
        let usage = self.quota_cache.calculate_usage_async(&self.home_dir).await;
        let quota_bytes = limit.saturating_mul(1024 * 1024);
        usage.saturating_add(additional) <= quota_bytes
    }

    async fn throttle(&self, bytes: usize) {
        if let Some(ref limiter) = self.speed_limiter {
            limiter.throttle(bytes).await;
        }
    }

    fn status(id: u32, code: StatusCode) -> Status {
        Status {
            id,
            status_code: code,
            error_message: code.to_string(),
            language_tag: "en-US".to_string(),
        }
    }

    fn io_status(e: &std::io::Error) -> StatusCode {
        use std::io::ErrorKind::{NotFound, PermissionDenied};
        match e.kind() {
            NotFound => StatusCode::NoSuchFile,
            PermissionDenied => StatusCode::PermissionDenied,
            _ => StatusCode::Failure,
        }
    }

    fn attrs_from(md: &std::fs::Metadata) -> FileAttributes {
        use std::os::unix::fs::MetadataExt;
        FileAttributes {
            size: Some(md.len()),
            uid: Some(md.uid()),
            user: None,
            gid: Some(md.gid()),
            group: None,
            permissions: Some(md.mode()),
            atime: Some(
                u32::try_from(md.atime().clamp(0, i64::from(u32::MAX))).unwrap_or_default(),
            ),
            mtime: Some(
                u32::try_from(md.mtime().clamp(0, i64::from(u32::MAX))).unwrap_or_default(),
            ),
        }
    }

    async fn stat_attrs(path: &PathBuf) -> SftpResult<FileAttributes> {
        let md = tokio::fs::metadata(path)
            .await
            .map_err(|_| StatusCode::NoSuchFile)?;
        Ok(Self::attrs_from(&md))
    }

    async fn symlink_attrs(path: &PathBuf) -> SftpResult<FileAttributes> {
        let md = tokio::fs::symlink_metadata(path)
            .await
            .map_err(|_| StatusCode::NoSuchFile)?;
        Ok(Self::attrs_from(&md))
    }
}
