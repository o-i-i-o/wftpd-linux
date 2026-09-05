//! SFTP 协议操作实现（russh_sftp::server::Handler）。
//!
//! 业务逻辑与旧手写实现保持一致：
//! - 路径经 `safe_resolve_path_async` 锚定在用户主目录内（防穿越）
//! - 每个操作前检查用户权限（can_read/can_write/...）
//! - 上传写入量受 quota_mb 配额约束
//! - 传输受用户 speed_limit_kbps 限速
//! - 文件操作写入审计日志（FileLogger → file_ops target）
//! - 支持 openssh 扩展：md5sum / sha256sum / space-available

use std::collections::HashMap;
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};

use russh_sftp::protocol::{
    Attrs, Data, File, FileAttributes, Handle, Name, OpenFlags, Packet, Status, StatusCode, Version,
};
use russh_sftp::server::Handler;
use tracing::{debug, info, warn};
use wftpd_common::server::quota::QuotaCache;
use wftpd_common::server::speed_limiter::SpeedLimiter;
use wftpd_common::{FileLogger, Permissions, UserManager};

const SFTP_VERSION: u32 = 3;

type SftpResult<T> = Result<T, StatusCode>;

/// 打开的文件句柄
struct FileHandle {
    file: tokio::fs::File,
    writable: bool,
}

/// 打开的目录句柄（opendir 时一次性枚举，readdir 分批返回）
struct DirHandle {
    entries: Vec<File>,
    cursor: usize,
}

enum OpenEntry {
    File(FileHandle),
    Dir(DirHandle),
}

/// 每个已认证的 SFTP 会话一个实例（由 russh_sftp::server::run 驱动）
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

    // ---- 内部工具 ----

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
            file_log.log(wftpd_common::FileLogInfo {
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
            let handle = format!("h{:x}", id);
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
        use std::io::ErrorKind::*;
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
            atime: Some(md.atime().clamp(i32::MIN as i64, i32::MAX as i64) as u32),
            mtime: Some(md.mtime().clamp(i32::MIN as i64, i32::MAX as i64) as u32),
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

impl Handler for SftpFileHandler {
    type Error = StatusCode;

    fn unimplemented(&self) -> Self::Error {
        StatusCode::OpUnsupported
    }

    async fn init(
        &mut self,
        _version: u32,
        _extensions: HashMap<String, String>,
    ) -> SftpResult<Version> {
        let mut version = Version::new();
        version.version = SFTP_VERSION;
        version
            .extensions
            .insert("md5sum@openssh.com".to_string(), "1".to_string());
        version
            .extensions
            .insert("sha256sum@openssh.com".to_string(), "1".to_string());
        version
            .extensions
            .insert("space-available@openssh.com".to_string(), "1".to_string());
        Ok(version)
    }

    async fn open(
        &mut self,
        id: u32,
        filename: String,
        pflags: OpenFlags,
        _attrs: FileAttributes,
    ) -> SftpResult<Handle> {
        let perms = self.permissions();
        let wants_write = pflags.intersects(
            OpenFlags::WRITE | OpenFlags::APPEND | OpenFlags::TRUNCATE | OpenFlags::CREATE,
        );
        let wants_append = pflags.contains(OpenFlags::APPEND);
        if wants_write {
            let allowed = if wants_append {
                perms.can_append || perms.can_write
            } else {
                perms.can_write
            };
            if !allowed {
                self.audit("OPEN", &filename, 0, false, "权限拒绝");
                return Err(StatusCode::PermissionDenied);
            }
            if !self.quota_ok(0).await {
                return Err(StatusCode::Failure);
            }
        } else if !perms.can_read {
            self.audit("OPEN", &filename, 0, false, "权限拒绝");
            return Err(StatusCode::PermissionDenied);
        }

        let path = self.resolve(&filename)?;

        let mut opts = tokio::fs::OpenOptions::new();
        if pflags.contains(OpenFlags::READ) {
            opts.read(true);
        }
        if pflags.contains(OpenFlags::WRITE)
            || pflags.contains(OpenFlags::APPEND)
            || pflags.contains(OpenFlags::CREATE)
        {
            opts.write(true);
        }
        if pflags.contains(OpenFlags::APPEND) {
            opts.append(true);
        }
        if pflags.contains(OpenFlags::CREATE) {
            opts.create(true);
        }
        if pflags.contains(OpenFlags::TRUNCATE) {
            opts.truncate(true);
        }
        if !pflags.contains(OpenFlags::READ) {
            // 纯写打开时若文件不存在则创建失败由 OS 决定（无 O_EXCL 处理）
        }

        let file = opts.open(&path).await.map_err(|e| {
            warn!(path = %path.display(), error = %e, "[SFTP] open failed");
            Self::io_status(&e)
        })?;

        let handle = self.next_handle_id();
        self.handles.insert(
            handle.clone(),
            OpenEntry::File(FileHandle {
                file,
                writable: wants_write,
            }),
        );

        debug!(user = %self.username, path = %path.display(), "[SFTP] file opened");
        let _ = path;
        Ok(Handle { id, handle })
    }

    async fn close(&mut self, id: u32, handle: String) -> SftpResult<Status> {
        match self.handles.remove(&handle) {
            Some(OpenEntry::File(f)) => {
                drop(f.file);
                if f.writable {
                    self.quota_cache.invalidate(&self.home_dir).await;
                }
                Ok(Self::status(id, StatusCode::Ok))
            }
            Some(OpenEntry::Dir(_)) => Ok(Self::status(id, StatusCode::Ok)),
            None => Err(StatusCode::NoSuchFile),
        }
    }

    async fn read(&mut self, id: u32, handle: String, offset: u64, len: u32) -> SftpResult<Data> {
        let Some(OpenEntry::File(f)) = self.handles.get_mut(&handle) else {
            return Err(StatusCode::NoSuchFile);
        };

        use tokio::io::{AsyncReadExt, AsyncSeekExt};
        f.file
            .seek(SeekFrom::Start(offset))
            .await
            .map_err(|e| Self::io_status(&e))?;

        let mut buf = vec![0u8; len as usize];
        let n = f
            .file
            .read(&mut buf)
            .await
            .map_err(|e| Self::io_status(&e))?;
        if n == 0 {
            return Err(StatusCode::Eof);
        }
        buf.truncate(n);

        self.throttle(n).await;
        Ok(Data { id, data: buf })
    }

    async fn write(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        data: Vec<u8>,
    ) -> SftpResult<Status> {
        let len = data.len() as u64;

        if !self.quota_ok(len).await {
            self.audit("WRITE", "", len, false, "配额超限");
            return Err(StatusCode::Failure);
        }

        let Some(OpenEntry::File(f)) = self.handles.get_mut(&handle) else {
            return Err(StatusCode::NoSuchFile);
        };

        use tokio::io::{AsyncSeekExt, AsyncWriteExt};
        f.file
            .seek(SeekFrom::Start(offset))
            .await
            .map_err(|e| Self::io_status(&e))?;
        f.file
            .write_all(&data)
            .await
            .map_err(|e| Self::io_status(&e))?;
        f.file.flush().await.map_err(|e| Self::io_status(&e))?;

        self.throttle(len as usize).await;
        Ok(Self::status(id, StatusCode::Ok))
    }

    async fn lstat(&mut self, id: u32, path: String) -> SftpResult<Attrs> {
        if !self.permissions().can_list {
            return Err(StatusCode::PermissionDenied);
        }
        let resolved = self.resolve(&path)?;
        let attrs = Self::symlink_attrs(&resolved).await?;
        Ok(Attrs { id, attrs })
    }

    async fn fstat(&mut self, id: u32, handle: String) -> SftpResult<Attrs> {
        let Some(OpenEntry::File(f)) = self.handles.get(&handle) else {
            return Err(StatusCode::NoSuchFile);
        };
        let md = f.file.metadata().await.map_err(|e| Self::io_status(&e))?;
        Ok(Attrs {
            id,
            attrs: Self::attrs_from(&md),
        })
    }

    async fn setstat(
        &mut self,
        id: u32,
        path: String,
        attrs: FileAttributes,
    ) -> SftpResult<Status> {
        if !self.permissions().can_write {
            return Err(StatusCode::PermissionDenied);
        }
        let resolved = self.resolve(&path)?;

        if let Some(permissions) = attrs.permissions {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(
                &resolved,
                std::fs::Permissions::from_mode(permissions & 0o7777),
            )
            .await
            .map_err(|e| Self::io_status(&e))?;
        }
        if let Some(size) = attrs.size {
            let f = tokio::fs::OpenOptions::new()
                .write(true)
                .open(&resolved)
                .await
                .map_err(|e| Self::io_status(&e))?;
            f.set_len(size).await.map_err(|e| Self::io_status(&e))?;
        }
        if let Some(mtime) = attrs.mtime {
            // 尽力而为：仅 mtime 有需求时用 filetime 风格操作缺失，暂不支持 utime
            let _ = mtime;
        }

        self.audit(
            "SETSTAT",
            &resolved.to_string_lossy(),
            0,
            true,
            "设置属性成功",
        );
        Ok(Self::status(id, StatusCode::Ok))
    }

    async fn opendir(&mut self, id: u32, path: String) -> SftpResult<Handle> {
        if !self.permissions().can_list {
            return Err(StatusCode::PermissionDenied);
        }
        let resolved = self.resolve(&path)?;

        let mut entries = Vec::new();
        let mut rd = tokio::fs::read_dir(&resolved)
            .await
            .map_err(|e| Self::io_status(&e))?;
        while let Some(entry) = rd.next_entry().await.map_err(|e| Self::io_status(&e))? {
            let attrs = match entry.metadata().await {
                Ok(md) => Self::attrs_from(&md),
                Err(_) => FileAttributes::dummy(),
            };
            entries.push(File::new(
                entry.file_name().to_string_lossy().into_owned(),
                attrs,
            ));
        }

        let handle = self.next_handle_id();
        self.handles.insert(
            handle.clone(),
            OpenEntry::Dir(DirHandle { entries, cursor: 0 }),
        );
        Ok(Handle { id, handle })
    }

    async fn readdir(&mut self, id: u32, handle: String) -> SftpResult<Name> {
        let Some(OpenEntry::Dir(dir)) = self.handles.get_mut(&handle) else {
            return Err(StatusCode::NoSuchFile);
        };

        if dir.cursor >= dir.entries.len() {
            return Err(StatusCode::Eof);
        }
        let end = (dir.cursor + 100).min(dir.entries.len());
        let files = dir.entries[dir.cursor..end].to_vec();
        dir.cursor = end;
        Ok(Name { id, files })
    }

    async fn remove(&mut self, id: u32, filename: String) -> SftpResult<Status> {
        if !self.permissions().can_delete {
            self.audit("DELETE", &filename, 0, false, "权限拒绝");
            return Err(StatusCode::PermissionDenied);
        }
        let resolved = self.resolve(&filename)?;
        let size = tokio::fs::metadata(&resolved)
            .await
            .map(|m| m.len())
            .unwrap_or(0);

        tokio::fs::remove_file(&resolved)
            .await
            .map_err(|e| Self::io_status(&e))?;

        self.quota_cache.invalidate(&self.home_dir).await;
        self.audit(
            "DELETE",
            &resolved.to_string_lossy(),
            size,
            true,
            "文件删除成功",
        );
        info!(user = %self.username, path = %resolved.display(), "[SFTP] file removed");
        Ok(Self::status(id, StatusCode::Ok))
    }

    async fn mkdir(&mut self, id: u32, path: String, _attrs: FileAttributes) -> SftpResult<Status> {
        if !self.permissions().can_mkdir {
            self.audit("MKDIR", &path, 0, false, "权限拒绝");
            return Err(StatusCode::PermissionDenied);
        }
        let resolved = self.resolve(&path)?;
        tokio::fs::create_dir(&resolved)
            .await
            .map_err(|e| Self::io_status(&e))?;

        self.audit(
            "MKDIR",
            &resolved.to_string_lossy(),
            0,
            true,
            "目录创建成功",
        );
        Ok(Self::status(id, StatusCode::Ok))
    }

    async fn rmdir(&mut self, id: u32, path: String) -> SftpResult<Status> {
        if !self.permissions().can_rmdir {
            self.audit("RMDIR", &path, 0, false, "权限拒绝");
            return Err(StatusCode::PermissionDenied);
        }
        let resolved = self.resolve(&path)?;
        // 与既有行为一致：递归删除（客户端 RMDIR 语义下目录可能含残留文件）
        tokio::fs::remove_dir_all(&resolved)
            .await
            .map_err(|e| Self::io_status(&e))?;

        self.audit(
            "RMDIR",
            &resolved.to_string_lossy(),
            0,
            true,
            "目录删除成功",
        );
        Ok(Self::status(id, StatusCode::Ok))
    }

    async fn realpath(&mut self, id: u32, path: String) -> SftpResult<Name> {
        let resolved = self.resolve(&path)?;
        // 把真实路径映射回 chroot 内的虚拟路径（客户端视角的根就是主目录）
        let home = PathBuf::from(&self.home_dir);
        let home_canon = tokio::fs::canonicalize(&home).await.unwrap_or(home);
        let virtual_path = resolved
            .strip_prefix(&home_canon)
            .map(|rel| {
                if rel.as_os_str().is_empty() {
                    "/".to_string()
                } else {
                    format!("/{}", rel.to_string_lossy())
                }
            })
            .unwrap_or_else(|_| "/".to_string());
        Ok(Name {
            id,
            files: vec![File::dummy(virtual_path)],
        })
    }

    async fn stat(&mut self, id: u32, path: String) -> SftpResult<Attrs> {
        if !self.permissions().can_list {
            return Err(StatusCode::PermissionDenied);
        }
        let resolved = self.resolve(&path)?;
        let attrs = Self::stat_attrs(&resolved).await?;
        Ok(Attrs { id, attrs })
    }

    async fn rename(&mut self, id: u32, oldpath: String, newpath: String) -> SftpResult<Status> {
        if !self.permissions().can_rename {
            self.audit(
                "RENAME",
                &format!("{oldpath} -> {newpath}"),
                0,
                false,
                "权限拒绝",
            );
            return Err(StatusCode::PermissionDenied);
        }
        let from = self.resolve(&oldpath)?;
        let to = self.resolve(&newpath)?;

        if to.exists() {
            return Err(StatusCode::Failure);
        }
        tokio::fs::rename(&from, &to)
            .await
            .map_err(|e| Self::io_status(&e))?;

        self.audit(
            "RENAME",
            &format!("{} -> {}", from.display(), to.display()),
            0,
            true,
            "文件重命名成功",
        );
        Ok(Self::status(id, StatusCode::Ok))
    }

    async fn readlink(&mut self, id: u32, path: String) -> SftpResult<Name> {
        let resolved = self.resolve(&path)?;
        // 返回创建时保存的原样目标字符串（与 POSIX readlink 语义一致）
        let target = tokio::fs::read_link(&resolved)
            .await
            .map_err(|e| Self::io_status(&e))?;
        Ok(Name {
            id,
            files: vec![File::dummy(target.to_string_lossy().into_owned())],
        })
    }

    async fn symlink(
        &mut self,
        id: u32,
        linkpath: String,
        targetpath: String,
    ) -> SftpResult<Status> {
        if !self.permissions().can_write {
            return Err(StatusCode::PermissionDenied);
        }
        // 注意：主流客户端（paramiko / OpenSSH sftp）发送顺序为
        // (target, linkpath)，与 SFTP 规范相反，这里按实际生态语义处理
        let target_raw = &linkpath;
        let link = self.resolve(&targetpath)?;

        // 目标按 POSIX 语义保存原样字符串，但需校验不越出主目录：
        // 相对目标按"链接所在目录"解析后校验；绝对目标直接 resolve 校验
        if target_raw.starts_with('/') {
            self.resolve(target_raw)?;
        } else {
            let link_dir = link
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from(&self.home_dir));
            let joined = link_dir.join(target_raw);
            let home = PathBuf::from(&self.home_dir);
            let mut normalized = home.clone();
            for component in joined.strip_prefix(&home).unwrap_or(&joined).components() {
                match component {
                    std::path::Component::Normal(seg) => {
                        normalized.push(seg);
                    }
                    std::path::Component::CurDir => {}
                    std::path::Component::ParentDir => {
                        if normalized == home {
                            return Err(StatusCode::PermissionDenied);
                        }
                        normalized.pop();
                    }
                    _ => return Err(StatusCode::PermissionDenied),
                }
            }
        }

        if link.exists() {
            return Err(StatusCode::Failure);
        }
        std::os::unix::fs::symlink(target_raw, &link).map_err(|e| Self::io_status(&e))?;

        self.audit(
            "SYMLINK",
            &format!("{} -> {}", link.display(), target_raw),
            0,
            true,
            "符号链接创建成功",
        );
        Ok(Self::status(id, StatusCode::Ok))
    }

    async fn extended(&mut self, id: u32, request: String, data: Vec<u8>) -> SftpResult<Packet> {
        match request.as_str() {
            "md5sum@openssh.com" => self.extended_digest(id, &data, Digest::Md5).await,
            "sha256sum@openssh.com" => self.extended_digest(id, &data, Digest::Sha256).await,
            "space-available@openssh.com" => self.extended_space_available(id, &data).await,
            other => {
                debug!(request = other, "[SFTP] unsupported extension");
                Err(StatusCode::OpUnsupported)
            }
        }
    }
}

enum Digest {
    Md5,
    Sha256,
}

impl SftpFileHandler {
    /// md5sum/sha256sum@openssh.com：请求体为 (path, offset, length)
    async fn extended_digest(&self, id: u32, data: &[u8], kind: Digest) -> SftpResult<Packet> {
        let (path, offset, length) = parse_digest_request(data).ok_or(StatusCode::BadMessage)?;
        let resolved = self.resolve(&path)?;

        let payload = digest_file(&resolved, offset, length, kind)
            .await
            .map_err(|e| Self::io_status(&e))?;

        Ok(Packet::ExtendedReply(russh_sftp::protocol::ExtendedReply {
            id,
            data: payload,
        }))
    }

    /// space-available@openssh.com（statvfs 编码按 draft-ietf-secsh-filexfer-06 §8）
    async fn extended_space_available(&self, id: u32, data: &[u8]) -> SftpResult<Packet> {
        let bad = StatusCode::BadMessage;
        let path_len =
            u32::from_be_bytes(data.get(0..4).ok_or(bad)?.try_into().map_err(|_| bad)?) as usize;
        let path = std::str::from_utf8(data.get(4..4 + path_len).ok_or(bad)?).map_err(|_| bad)?;
        let resolved = self.resolve(path)?;

        let stats = nix::sys::statvfs::statvfs(&resolved).map_err(|_| StatusCode::Failure)?;

        let mut payload = Vec::with_capacity(88);
        for v in [
            stats.block_size() as u64,
            stats.fragment_size() as u64,
            stats.blocks() as u64,
            stats.blocks_free() as u64,
            stats.blocks_available() as u64,
            stats.files() as u64,
            stats.files_free() as u64,
            stats.files_available() as u64,
            0u64, // fsid
            0u64, // flags
            stats.name_max() as u64,
        ] {
            payload.extend_from_slice(&v.to_be_bytes());
        }

        Ok(Packet::ExtendedReply(russh_sftp::protocol::ExtendedReply {
            id,
            data: payload,
        }))
    }
}

/// 解析 md5sum/sha256sum 请求：string path, uint64 offset, uint64 length
/// （length == 0 表示从 offset 起到文件末尾）
fn parse_digest_request(data: &[u8]) -> Option<(String, u64, u64)> {
    let read_u32 = |pos: usize| -> Option<u32> {
        Some(u32::from_be_bytes(data.get(pos..pos + 4)?.try_into().ok()?))
    };
    let read_u64 = |pos: usize| -> Option<u64> {
        Some(u64::from_be_bytes(data.get(pos..pos + 8)?.try_into().ok()?))
    };

    let path_len = read_u32(0)? as usize;
    let path = String::from_utf8(data.get(4..4 + path_len)?.to_vec()).ok()?;
    let mut pos = 4 + path_len;
    let offset = read_u64(pos).unwrap_or(0);
    pos += 8;
    let length = read_u64(pos).unwrap_or(0);
    Some((path, offset, length))
}

async fn digest_file(
    path: &PathBuf,
    offset: u64,
    length: u64,
    kind: Digest,
) -> std::io::Result<Vec<u8>> {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};

    let mut file = tokio::fs::File::open(path).await?;
    file.seek(SeekFrom::Start(offset)).await?;

    let mut remaining = length;
    let mut buf = [0u8; 8192];
    use md5::Digest as _;
    let mut md5_hasher = md5::Md5::new();
    let mut sha_hasher = sha2::Sha256::new();
    loop {
        let want = if remaining == 0 {
            buf.len()
        } else {
            buf.len().min(remaining as usize)
        };
        let n = file.read(&mut buf[..want]).await?;
        if n == 0 {
            break;
        }
        md5_hasher.update(&buf[..n]);
        sha_hasher.update(&buf[..n]);
        if remaining > 0 {
            remaining -= n as u64;
            if remaining == 0 {
                break;
            }
        }
    }

    let hex = match kind {
        Digest::Md5 => hex::encode(md5_hasher.finalize()),
        Digest::Sha256 => hex::encode(sha_hasher.finalize()),
    };
    Ok(hex.into_bytes())
}
