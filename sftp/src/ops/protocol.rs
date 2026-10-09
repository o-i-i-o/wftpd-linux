//! SFTP 协议方法实现（`russh_sftp::server::Handler`）。
//!
//! Rust 要求同一 trait 的 impl 必须是单个块，因此本文件聚合全部协议方法；
//! 按操作类别划分的辅助逻辑位于 [`super::extended`] 等子模块。

use std::collections::HashMap;
use std::io::SeekFrom;
use std::path::{Path, PathBuf};

use russh_sftp::protocol::{
    Attrs, Data, File, FileAttributes, Handle, Name, OpenFlags, Packet, Status, StatusCode, Version,
};
use russh_sftp::server::Handler;
use tracing::{debug, info, warn};

use super::extended::Digest;
use super::{
    DirHandle, FileHandle, MAX_READ_LEN, OpenEntry, SFTP_VERSION, SftpFileHandler, SftpResult,
};

impl Handler for SftpFileHandler {
    type Error = StatusCode;

    fn unimplemented(&self) -> Self::Error {
        StatusCode::OpUnsupported
    }

    // ---- 会话与文件句柄（INIT/OPEN/CLOSE/READ/WRITE） ----

    async fn init(
        &mut self,
        _version: u32,
        _extensions: HashMap<String, String>,
    ) -> SftpResult<Version> {
        // 版本协商无 IO 可等待；显式让步一次，避免初始化处理独占调度
        tokio::task::yield_now().await;
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
        use tokio::io::{AsyncReadExt, AsyncSeekExt};

        let Some(OpenEntry::File(f)) = self.handles.get_mut(&handle) else {
            return Err(StatusCode::NoSuchFile);
        };

        f.file
            .seek(SeekFrom::Start(offset))
            .await
            .map_err(|e| Self::io_status(&e))?;

        let len = len.min(MAX_READ_LEN);
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
        use tokio::io::{AsyncSeekExt, AsyncWriteExt};

        let len = data.len() as u64;

        if !self.quota_ok(len).await {
            self.audit("WRITE", "", len, false, "配额超限");
            return Err(StatusCode::Failure);
        }

        let Some(OpenEntry::File(f)) = self.handles.get_mut(&handle) else {
            return Err(StatusCode::NoSuchFile);
        };

        f.file
            .seek(SeekFrom::Start(offset))
            .await
            .map_err(|e| Self::io_status(&e))?;
        f.file
            .write_all(&data)
            .await
            .map_err(|e| Self::io_status(&e))?;
        f.file.flush().await.map_err(|e| Self::io_status(&e))?;

        self.throttle(data.len()).await;
        Ok(Self::status(id, StatusCode::Ok))
    }

    // ---- 元数据与目录（STAT/LSTAT/FSTAT/SETSTAT/OPENDIR/READDIR） ----

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

        let rd = tokio::fs::read_dir(&resolved)
            .await
            .map_err(|e| Self::io_status(&e))?;

        let handle = self.next_handle_id();
        self.handles
            .insert(handle.clone(), OpenEntry::Dir(DirHandle { rd }));
        Ok(Handle { id, handle })
    }

    async fn readdir(&mut self, id: u32, handle: String) -> SftpResult<Name> {
        const BATCH: usize = 100;

        let Some(OpenEntry::Dir(dir)) = self.handles.get_mut(&handle) else {
            return Err(StatusCode::NoSuchFile);
        };

        let mut files = Vec::new();
        while files.len() < BATCH {
            match dir.rd.next_entry().await.map_err(|e| Self::io_status(&e))? {
                Some(entry) => {
                    let attrs = match entry.metadata().await {
                        Ok(md) => Self::attrs_from(&md),
                        Err(_) => FileAttributes::dummy(),
                    };
                    files.push(File::new(
                        entry.file_name().to_string_lossy().into_owned(),
                        attrs,
                    ));
                }
                None => break,
            }
        }

        if files.is_empty() {
            return Err(StatusCode::Eof);
        }
        Ok(Name { id, files })
    }

    // ---- 文件与目录增删改名（REMOVE/MKDIR/RMDIR/REALPATH/RENAME） ----

    async fn remove(&mut self, id: u32, filename: String) -> SftpResult<Status> {
        if !self.permissions().can_delete {
            self.audit("DELETE", &filename, 0, false, "权限拒绝");
            return Err(StatusCode::PermissionDenied);
        }
        let resolved = self.resolve(&filename)?;
        let size = tokio::fs::metadata(&resolved).await.map_or(0, |m| m.len());

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
        let virtual_path = resolved.strip_prefix(&home_canon).map_or_else(
            |_| "/".to_string(),
            |rel| {
                if rel.as_os_str().is_empty() {
                    "/".to_string()
                } else {
                    format!("/{}", rel.to_string_lossy())
                }
            },
        );
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

    // ---- 符号链接与 openssh 扩展（READLINK/SYMLINK/EXTENDED） ----

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
                .map_or_else(|| PathBuf::from(&self.home_dir), Path::to_path_buf);
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
        tokio::fs::symlink(target_raw, &link)
            .await
            .map_err(|e| Self::io_status(&e))?;

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
            "space-available@openssh.com" => self.extended_space_available(id, &data),
            other => {
                debug!(request = other, "[SFTP] unsupported extension");
                Err(StatusCode::OpUnsupported)
            }
        }
    }
}
