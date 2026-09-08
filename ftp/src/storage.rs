//! libunftp 存储后端：按用户主目录隔离的文件系统访问。
//!
//! [`WftpdUser`] 实现 `UserDetail`（声明主目录，libunftp 据此限制会话根），
//! [`WftpdFilesystem`] 在每个操作中执行：
//! - 路径安全（拒绝越出主目录）
//! - `用户权限（can_read/can_write/can_delete/can_list/can_mkdir/can_rmdir/can_rename`）
//! - `配额（quota_mb，写入前检查`）
//! - 审计日志（FileLogger → `file_ops` target / 内存缓冲 → 前端）

use std::fmt::Debug;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};

use async_trait::async_trait;
use tracing::debug;
use unftp_core::auth::{DefaultUser, UserDetail};
use unftp_core::storage::{
    Error as StorageError, ErrorKind, Fileinfo, Metadata, Permissions, Result as StorageResult,
    StorageBackend,
};
use wftpd_common::FileLogger;
use wftpd_common::server::quota::QuotaCache;

/// libunftp 会话的用户上下文
#[derive(Debug)]
pub struct WftpdUser {
    pub username: String,
    pub home: PathBuf,
    pub permissions: wftpd_common::Permissions,
    pub quota_mb: u64,
    pub anonymous: bool,
}

impl WftpdUser {
    #[must_use]
    pub fn new(username: String, home: PathBuf, permissions: wftpd_common::Permissions) -> Self {
        let quota_mb = permissions.quota_mb.unwrap_or(0);
        WftpdUser {
            username,
            home,
            permissions,
            quota_mb,
            anonymous: false,
        }
    }

    #[must_use]
    pub fn anonymous(home: PathBuf) -> Self {
        WftpdUser {
            username: "anonymous".to_string(),
            home,
            permissions: wftpd_common::Permissions::full(),
            quota_mb: 0,
            anonymous: true,
        }
    }
}

impl UserDetail for WftpdUser {
    fn home(&self) -> Option<&Path> {
        Some(&self.home)
    }
}

impl std::fmt::Display for WftpdUser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.username)
    }
}

/// 在 `StorageBackend` 的 blanket impl 中统一提取用户信息。
/// `DefaultUser` 分支只为满足 libunftp 构造器的类型约束，运行时不会出现。
trait SessionUser {
    fn context(&self) -> Option<SessionCtx<'_>>;
}

struct SessionCtx<'a> {
    username: &'a str,
    permissions: &'a wftpd_common::Permissions,
    quota_mb: u64,
}

impl SessionUser for WftpdUser {
    fn context(&self) -> Option<SessionCtx<'_>> {
        Some(SessionCtx {
            username: &self.username,
            permissions: &self.permissions,
            quota_mb: self.quota_mb,
        })
    }
}

impl SessionUser for DefaultUser {
    fn context(&self) -> Option<SessionCtx<'_>> {
        None
    }
}

/// 文件系统存储后端（每个 libunftp 会话一个实例）。
///
/// libunftp 在登录成功后调用 [`StorageBackend::enter`]，此处把会话根切换到
/// 用户主目录；后续所有路径都相对该根解析并做越界检查（等价 chroot）。
pub struct WftpdFilesystem {
    root: StdMutex<Option<PathBuf>>,
    file_logger: Arc<StdMutex<FileLogger>>,
    quota_cache: Arc<QuotaCache>,
}

impl Debug for WftpdFilesystem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WftpdFilesystem").finish()
    }
}

impl WftpdFilesystem {
    pub fn new(file_logger: Arc<StdMutex<FileLogger>>, quota_cache: Arc<QuotaCache>) -> Self {
        WftpdFilesystem {
            root: StdMutex::new(None),
            file_logger,
            quota_cache,
        }
    }

    fn audit(&self, user: &str, op: &str, path: &Path, size: u64, success: bool, message: &str) {
        if let Ok(mut log) = self.file_logger.try_lock() {
            log.log(&wftpd_common::FileLogInfo {
                username: user,
                client_ip: "-",
                operation: op,
                file_path: &path.to_string_lossy(),
                file_size: size,
                protocol: "FTP",
                success,
                message,
            });
        }
    }

    /// 相对会话根解析路径；拒绝越出根目录的 `..` 穿越
    fn resolve(&self, path: &Path) -> StorageResult<PathBuf> {
        let root = self.root.lock().unwrap().clone();
        let Some(root) = root else {
            return Err(ErrorKind::PermanentFileNotAvailable.into());
        };

        let stripped = path.strip_prefix("/").unwrap_or(path);
        let mut normalized = root.clone();
        for component in stripped.components() {
            match component {
                std::path::Component::Normal(seg) => normalized.push(seg),
                std::path::Component::CurDir => {}
                std::path::Component::ParentDir => {
                    if normalized == root {
                        return Err(ErrorKind::FileNameNotAllowedError.into());
                    }
                    normalized.pop();
                }
                _ => return Err(ErrorKind::FileNameNotAllowedError.into()),
            }
        }
        Ok(normalized)
    }

    fn require(cond: bool, op: &str, user: &str) -> StorageResult<()> {
        if cond {
            Ok(())
        } else {
            debug!(user = %user, operation = op, "FTP 操作被权限规则拒绝");
            Err(ErrorKind::PermissionDenied.into())
        }
    }

    async fn check_quota(&self, home: &Path, quota_mb: u64) -> StorageResult<()> {
        if quota_mb == 0 {
            return Ok(());
        }
        let Some(home_str) = home.to_str() else {
            return Ok(());
        };
        let usage = self.quota_cache.calculate_usage_async(home_str).await;
        let quota_bytes = quota_mb.saturating_mul(1024 * 1024);
        if usage >= quota_bytes {
            return Err(ErrorKind::InsufficientStorageSpaceError.into());
        }
        Ok(())
    }

    fn current_root(&self) -> StorageResult<PathBuf> {
        self.root
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| ErrorKind::PermanentFileNotAvailable.into())
    }
}

#[async_trait]
impl<UD> StorageBackend<UD> for WftpdFilesystem
where
    UD: UserDetail + SessionUser + Send + Sync,
{
    type Metadata = SystemMetadata;

    fn enter(&mut self, user_detail: &UD) -> std::io::Result<()> {
        if let Some(home) = user_detail.home() {
            let home = home.to_path_buf();
            std::fs::create_dir_all(&home)?;
            let canon = std::fs::canonicalize(&home)?;
            *self.root.lock().unwrap() = Some(canon);
        }
        Ok(())
    }

    async fn metadata<P: AsRef<Path> + Send + Debug>(
        &self,
        _user: &UD,
        path: P,
    ) -> StorageResult<Self::Metadata> {
        let p = self.resolve(path.as_ref())?;
        let md = tokio::fs::symlink_metadata(&p)
            .await
            .map_err(|e| io_err(&e))?;
        Ok(SystemMetadata(md))
    }

    async fn list<P: AsRef<Path> + Send + Debug>(
        &self,
        user: &UD,
        path: P,
    ) -> StorageResult<Vec<Fileinfo<PathBuf, Self::Metadata>>> {
        let Some(ctx) = user.context() else {
            return Err(ErrorKind::PermanentFileNotAvailable.into());
        };
        Self::require(ctx.permissions.can_list, "LIST", ctx.username)?;
        let p = self.resolve(path.as_ref())?;
        let mut result = Vec::new();
        let mut rd = tokio::fs::read_dir(&p).await.map_err(|e| io_err(&e))?;
        while let Some(entry) = rd.next_entry().await.map_err(|e| io_err(&e))? {
            let md = entry.metadata().await.map_err(|e| io_err(&e))?;
            result.push(Fileinfo {
                path: entry.path(),
                metadata: SystemMetadata(md),
            });
        }
        Ok(result)
    }

    async fn get<P: AsRef<Path> + Send + Debug>(
        &self,
        user: &UD,
        path: P,
        start_pos: u64,
    ) -> StorageResult<Box<dyn tokio::io::AsyncRead + Send + Sync + Unpin>> {
        let Some(ctx) = user.context() else {
            return Err(ErrorKind::PermanentFileNotAvailable.into());
        };
        Self::require(ctx.permissions.can_read, "RETR", ctx.username)?;
        let p = self.resolve(path.as_ref())?;
        let mut file = tokio::fs::File::open(&p).await.map_err(|e| {
            self.audit(
                ctx.username,
                "DOWNLOAD",
                &p,
                0,
                false,
                &format!("下载失败: {e}"),
            );
            io_err(&e)
        })?;

        if start_pos > 0 {
            use tokio::io::AsyncSeekExt;
            file.seek(std::io::SeekFrom::Start(start_pos))
                .await
                .map_err(|e| io_err(&e))?;
        }

        let size = file.metadata().await.map_or(0, |m| m.len());
        self.audit(ctx.username, "DOWNLOAD", &p, size, true, "文件下载成功");
        Ok(Box::new(file))
    }

    async fn put<P, R>(&self, user: &UD, input: R, path: P, start_pos: u64) -> StorageResult<u64>
    where
        P: AsRef<Path> + Send + Debug,
        R: tokio::io::AsyncRead + Send + Sync + Unpin + 'static,
    {
        use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

        let Some(ctx) = user.context() else {
            return Err(ErrorKind::PermanentFileNotAvailable.into());
        };
        Self::require(ctx.permissions.can_write, "STOR", ctx.username)?;
        let p = self.resolve(path.as_ref())?;
        self.check_quota(&p, ctx.quota_mb).await?;

        if let Some(parent) = p.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| io_err(&e))?;
        }

        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(start_pos == 0)
            .open(&p)
            .await
            .map_err(|e| {
                self.audit(
                    ctx.username,
                    "UPLOAD",
                    &p,
                    0,
                    false,
                    &format!("上传失败: {e}"),
                );
                io_err(&e)
            })?;

        if start_pos > 0 {
            file.seek(std::io::SeekFrom::Start(start_pos))
                .await
                .map_err(|e| io_err(&e))?;
        }

        let mut input = input;
        let mut buffer = [0u8; 8192];
        let mut written: u64 = 0;
        loop {
            let n = input.read(&mut buffer).await.map_err(|e| io_err(&e))?;
            if n == 0 {
                break;
            }
            file.write_all(&buffer[..n]).await.map_err(|e| io_err(&e))?;
            written += n as u64;
        }
        file.flush().await.map_err(|e| io_err(&e))?;

        self.quota_cache
            .invalidate(&self.current_root()?.to_string_lossy())
            .await;
        self.audit(ctx.username, "UPLOAD", &p, written, true, "文件上传成功");
        Ok(written)
    }

    async fn del<P: AsRef<Path> + Send + Debug>(&self, user: &UD, path: P) -> StorageResult<()> {
        let Some(ctx) = user.context() else {
            return Err(ErrorKind::PermanentFileNotAvailable.into());
        };
        Self::require(ctx.permissions.can_delete, "DELE", ctx.username)?;
        let p = self.resolve(path.as_ref())?;
        tokio::fs::remove_file(&p).await.map_err(|e| io_err(&e))?;

        self.quota_cache
            .invalidate(&self.current_root()?.to_string_lossy())
            .await;
        self.audit(ctx.username, "DELETE", &p, 0, true, "文件删除成功");
        Ok(())
    }

    async fn mkd<P: AsRef<Path> + Send + Debug>(&self, user: &UD, path: P) -> StorageResult<()> {
        let Some(ctx) = user.context() else {
            return Err(ErrorKind::PermanentFileNotAvailable.into());
        };
        Self::require(ctx.permissions.can_mkdir, "MKD", ctx.username)?;
        let p = self.resolve(path.as_ref())?;
        tokio::fs::create_dir(&p).await.map_err(|e| io_err(&e))?;

        self.audit(ctx.username, "MKDIR", &p, 0, true, "目录创建成功");
        Ok(())
    }

    async fn rename<P: AsRef<Path> + Send + Debug>(
        &self,
        user: &UD,
        from: P,
        to: P,
    ) -> StorageResult<()> {
        let Some(ctx) = user.context() else {
            return Err(ErrorKind::PermanentFileNotAvailable.into());
        };
        Self::require(ctx.permissions.can_rename, "RNFR/RNTO", ctx.username)?;
        let from = self.resolve(from.as_ref())?;
        let to = self.resolve(to.as_ref())?;

        tokio::fs::rename(&from, &to)
            .await
            .map_err(|e| io_err(&e))?;

        self.audit(
            ctx.username,
            "RENAME",
            &from,
            0,
            true,
            &format!("重命名为 {}", to.display()),
        );
        Ok(())
    }

    async fn rmd<P: AsRef<Path> + Send + Debug>(&self, user: &UD, path: P) -> StorageResult<()> {
        let Some(ctx) = user.context() else {
            return Err(ErrorKind::PermanentFileNotAvailable.into());
        };
        Self::require(ctx.permissions.can_rmdir, "RMD", ctx.username)?;
        let p = self.resolve(path.as_ref())?;
        // 与既有行为一致：递归删除
        tokio::fs::remove_dir_all(&p)
            .await
            .map_err(|e| io_err(&e))?;

        self.audit(ctx.username, "RMDIR", &p, 0, true, "目录删除成功");
        Ok(())
    }

    async fn cwd<P: AsRef<Path> + Send + Debug>(&self, _user: &UD, path: P) -> StorageResult<()> {
        let p = self.resolve(path.as_ref())?;
        let md = tokio::fs::metadata(&p).await.map_err(|e| io_err(&e))?;
        if md.is_dir() {
            Ok(())
        } else {
            Err(ErrorKind::PermanentDirectoryNotAvailable.into())
        }
    }
}

/// `std::fs::Metadata` 的 Metadata trait 适配
#[derive(Clone)]
pub struct SystemMetadata(std::fs::Metadata);

impl Metadata for SystemMetadata {
    fn len(&self) -> u64 {
        self.0.len()
    }

    fn is_dir(&self) -> bool {
        self.0.is_dir()
    }

    fn is_file(&self) -> bool {
        self.0.is_file()
    }

    fn is_symlink(&self) -> bool {
        self.0.file_type().is_symlink()
    }

    fn modified(&self) -> Result<std::time::SystemTime, StorageError> {
        self.0
            .modified()
            .map_err(|e| StorageError::new(ErrorKind::PermanentFileNotAvailable, e))
    }

    fn gid(&self) -> u32 {
        use std::os::unix::fs::MetadataExt;
        self.0.gid()
    }

    fn uid(&self) -> u32 {
        use std::os::unix::fs::MetadataExt;
        self.0.uid()
    }

    fn permissions(&self) -> Permissions {
        use std::os::unix::fs::PermissionsExt;
        Permissions(self.0.permissions().mode())
    }
}

fn io_err(e: &std::io::Error) -> StorageError {
    match e.kind() {
        std::io::ErrorKind::NotFound | std::io::ErrorKind::AlreadyExists => {
            ErrorKind::PermanentFileNotAvailable.into()
        }
        std::io::ErrorKind::PermissionDenied => ErrorKind::PermissionDenied.into(),
        _ => ErrorKind::LocalError.into(),
    }
}

#[cfg(test)]
mod user_tests {
    use super::*;
    use wftpd_common::Permissions;

    #[test]
    fn new_user_defaults_to_zero_quota() {
        let user = WftpdUser::new(
            "alice".to_string(),
            PathBuf::from("/home/alice"),
            Permissions::full(),
        );
        assert!(!user.anonymous);
        assert_eq!(user.quota_mb, 0, "未配置 quota_mb 时归一为 0");
        assert_eq!(user.username, "alice");
    }

    #[test]
    fn new_user_carries_quota_from_permissions() {
        let perms = Permissions {
            quota_mb: Some(512),
            ..Permissions::full()
        };
        let user = WftpdUser::new("bob".to_string(), PathBuf::from("/home/bob"), perms);
        assert_eq!(user.quota_mb, 512);
    }

    #[test]
    fn anonymous_user_has_full_permissions() {
        let user = WftpdUser::anonymous(PathBuf::from("/srv/ftp"));
        assert!(user.anonymous);
        assert_eq!(user.username, "anonymous");
        assert_eq!(user.home, PathBuf::from("/srv/ftp"));
        assert_eq!(user.permissions, Permissions::full());
    }

    #[test]
    fn user_detail_home_and_display() {
        let user = WftpdUser::new(
            "carol".to_string(),
            PathBuf::from("/home/carol"),
            Permissions::default(),
        );
        assert_eq!(UserDetail::home(&user), Some(Path::new("/home/carol")));
        assert_eq!(user.to_string(), "carol");
    }
}
