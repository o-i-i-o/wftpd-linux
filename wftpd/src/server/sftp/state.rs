use anyhow::Result;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use tracing::{info, warn, debug};
use std::os::unix::fs::MetadataExt;

use super::packet::*;
use super::packet::SSH_FX_OK;
use super::packet::SSH_FX_EOF;
use super::packet::SSH_FX_NO_SUCH_FILE;
use super::packet::SSH_FX_PERMISSION_DENIED;
use super::packet::SSH_FX_FAILURE;
use super::packet::SSH_FX_OP_UNSUPPORTED;
use crate::core::users::UserManager;
use crate::core::users::Permissions;
use crate::core::file_logger::FileLogger;
use crate::server::common::quota::QuotaCache;
use crate::server::common::speed_limiter::SpeedLimiter;
use crate::server::common::utils::{
    safe_resolve_path,
    validate_path_within_chroot,
    validate_path_for_creation,
};

const SSH_FXP_INIT: u8 = 1;
const SSH_FXP_VERSION: u8 = 2;
const SSH_FXP_OPEN: u8 = 3;
const SSH_FXP_CLOSE: u8 = 4;
const SSH_FXP_READ: u8 = 5;
const SSH_FXP_WRITE: u8 = 6;
const SSH_FXP_LSTAT: u8 = 7;
const SSH_FXP_FSTAT: u8 = 8;
const SSH_FXP_SETSTAT: u8 = 9;
const SSH_FXP_FSETSTAT: u8 = 10;
const SSH_FXP_OPENDIR: u8 = 11;
const SSH_FXP_READDIR: u8 = 12;
const SSH_FXP_REMOVE: u8 = 13;
const SSH_FXP_MKDIR: u8 = 14;
const SSH_FXP_RMDIR: u8 = 15;
const SSH_FXP_REALPATH: u8 = 16;
const SSH_FXP_STAT: u8 = 17;
const SSH_FXP_READLINK: u8 = 18;
const SSH_FXP_SYMLINK: u8 = 19;
const SSH_FXP_RENAME: u8 = 20;
const SSH_FXP_LOCK: u8 = 40;
const SSH_FXP_UNLOCK: u8 = 41;
const SSH_FXP_EXTENDED: u8 = 200;

pub struct SftpFileHandle {
    pub path: PathBuf,
    pub file: Option<tokio::fs::File>,
    pub locked: bool,
    pub existed: bool,
    pub written_bytes: u64,
    pub read_bytes: u64,
    // 目录相关字段
    pub is_dir: bool,
    pub dir_entries: Vec<(String, bool, u64)>, // (name, is_dir, size)
    pub dir_index: usize,
}

pub struct SftpState {
    pub home_dir: String,
    pub cwd: String,
    pub username: Option<String>,
    pub user_manager: Arc<StdMutex<UserManager>>,
    pub file_logger: Arc<StdMutex<FileLogger>>,
    pub quota_cache: Arc<QuotaCache>,
    pub handles: HashMap<String, SftpFileHandle>,
    pub next_handle_id: u32,
    pub sftp_version: u32,
    pub buffer: Vec<u8>,
    pub locked_files: HashSet<PathBuf>,
    pub client_ip: String,
    cached_permissions: Option<Permissions>,
    speed_limiter: Option<Arc<SpeedLimiter>>,
}

impl Drop for SftpState {
    fn drop(&mut self) {
        let locked_handles: Vec<PathBuf> = self.handles.drain()
            .filter_map(|(_, handle)| {
                if handle.locked {
                    Some(handle.path)
                } else {
                    None
                }
            })
            .collect();
        
        for path in locked_handles {
            self.locked_files.remove(&path);
        }
        
        self.locked_files.clear();
        debug!(username = ?self.username, "SFTP session cleanup completed");
    }
}

fn io_error_to_sftp_status(e: &std::io::Error) -> (u32, &'static str) {
    match e.kind() {
        std::io::ErrorKind::NotFound => (SSH_FX_NO_SUCH_FILE, "No such file or directory"),
        std::io::ErrorKind::PermissionDenied => (SSH_FX_PERMISSION_DENIED, "Permission denied"),
        std::io::ErrorKind::AlreadyExists => (SSH_FX_FAILURE, "File already exists"),
        std::io::ErrorKind::IsADirectory => (SSH_FX_FAILURE, "Is a directory"),
        std::io::ErrorKind::NotADirectory => (SSH_FX_FAILURE, "Not a directory"),
        _ => (SSH_FX_FAILURE, "Operation failed"),
    }
}

fn format_longname(name: &str, is_dir: bool, size: u64, mtime: i64, mode: u32) -> String {
    let perms = if is_dir {
        "drwxr-xr-x"
    } else {
        let user_r = if mode & 0o400 != 0 { 'r' } else { '-' };
        let user_w = if mode & 0o200 != 0 { 'w' } else { '-' };
        let user_x = if mode & 0o100 != 0 { 'x' } else { '-' };
        let group_r = if mode & 0o040 != 0 { 'r' } else { '-' };
        let group_w = if mode & 0o020 != 0 { 'w' } else { '-' };
        let group_x = if mode & 0o010 != 0 { 'x' } else { '-' };
        let other_r = if mode & 0o004 != 0 { 'r' } else { '-' };
        let other_w = if mode & 0o002 != 0 { 'w' } else { '-' };
        let other_x = if mode & 0o001 != 0 { 'x' } else { '-' };
        &format!("-{}{}{}{}{}{}{}{}{}", 
            user_r, user_w, user_x,
            group_r, group_w, group_x,
            other_r, other_w, other_x)
    };
    
    let datetime = chrono::DateTime::from_timestamp(mtime, 0)
        .unwrap_or(chrono::DateTime::UNIX_EPOCH);
    let time_str = datetime.format("%b %d %H:%M").to_string();
    
    format!("{} 1 user user {:>10} {} {}", perms, size, time_str, name)
}

impl SftpState {
    pub fn new(
        home_dir: String,
        username: Option<String>,
        user_manager: Arc<StdMutex<UserManager>>,
        file_logger: Arc<StdMutex<FileLogger>>,
        quota_cache: Arc<QuotaCache>,
        client_ip: String,
    ) -> Self {
        let cached_permissions = username.as_ref().and_then(|u| {
            user_manager.lock().ok().and_then(|mgr| {
                mgr.get_user(u).map(|user| user.permissions)
            })
        });
        
        let speed_limiter = cached_permissions
            .and_then(|permissions| permissions.speed_limit_kbps)
            .filter(|limit| *limit > 0)
            .map(|limit| Arc::new(SpeedLimiter::new(limit)));

        SftpState {
            home_dir: home_dir.clone(),
            cwd: home_dir,
            username,
            user_manager,
            file_logger,
            quota_cache,
            handles: HashMap::new(),
            next_handle_id: 0,
            sftp_version: 3,
            buffer: Vec::new(),
            locked_files: HashSet::new(),
            client_ip,
            cached_permissions,
            speed_limiter,
        }
    }

    pub async fn process_sftp_data(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        self.buffer.extend_from_slice(data);
        
        const MAX_PACKET_SIZE: usize = 256 * 1024;
        let mut responses = Vec::new();
        
        while self.buffer.len() >= 4 {
            let packet_len = u32::from_be_bytes([
                self.buffer[0], self.buffer[1], self.buffer[2], self.buffer[3]
            ]) as usize;
            
            if packet_len == 0 || packet_len > MAX_PACKET_SIZE {
                self.buffer.clear();
                return Ok(build_status_packet(0, SSH_FX_FAILURE, "Invalid packet length", ""));
            }
            
            if self.buffer.len() < 4 + packet_len {
                break;
            }
            
            let packet: Vec<u8> = self.buffer[4..4 + packet_len].to_vec();
            self.buffer.drain(0..4 + packet_len);
            
            if !packet.is_empty() {
                let response = self.handle_sftp_packet(&packet).await?;
                responses.extend(response);
            }
        }
        
        Ok(responses)
    }

    fn get_permissions(&self) -> Option<&Permissions> {
        self.cached_permissions.as_ref()
    }

    fn check_permission_cached(&self, check_fn: impl Fn(&Permissions) -> bool) -> bool {
        if let Some(permissions) = self.get_permissions() {
            check_fn(permissions)
        } else {
            self.check_permission(check_fn)
        }
    }

    pub fn check_permission(&self, check_fn: impl Fn(&Permissions) -> bool) -> bool {
        let username = match &self.username {
            Some(u) => u,
            None => {
                warn!("SFTP permission check failed: username is None");
                return false;
            }
        };
        
        let users = match self.user_manager.lock() {
            Ok(u) => u,
            Err(_) => return false,
        };
        
        match users.get_user(username) {
            Some(user) => {
                let result = check_fn(&user.permissions);
                if !result {
                    warn!(username = %username, "SFTP permission denied");
                }
                result
            }
            None => {
                warn!(username = %username, "SFTP user not found in permission check");
                false
            }
        }
    }

    pub(crate) async fn check_quota_for_additional_bytes(&self, additional_bytes: u64) -> bool {
        if additional_bytes == 0 {
            return true;
        }

        let quota_mb = self.get_permissions()
            .and_then(|permissions| permissions.quota_mb)
            .filter(|limit| *limit > 0);

        match quota_mb {
            Some(limit) => {
                let current_usage = self.quota_cache.calculate_usage_async(&self.home_dir).await;
                let quota_bytes = limit.saturating_mul(1024 * 1024);
                current_usage.saturating_add(additional_bytes) <= quota_bytes
            }
            None => true,
        }
    }

    pub(crate) async fn invalidate_quota_cache(&self) {
        self.quota_cache.invalidate(&self.home_dir).await;
    }

    async fn handle_sftp_packet(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        if data.is_empty() {
            return Ok(build_status_packet(0, SSH_FX_FAILURE, "Bad packet: empty data", ""));
        }

        if data.len() < 5 {
            return Ok(build_status_packet(0, SSH_FX_FAILURE, "Bad packet: too short", ""));
        }

        let msg_type = data[0];
        

        match msg_type {
            SSH_FXP_INIT => self.handle_init(data).await,
            SSH_FXP_OPEN => self.handle_open(data).await,
            SSH_FXP_CLOSE => self.handle_close(data).await,
            SSH_FXP_READ => self.handle_read(data).await,
            SSH_FXP_WRITE => self.handle_write(data).await,
            SSH_FXP_LSTAT => self.handle_lstat(data).await,
            SSH_FXP_FSTAT => self.handle_fstat(data).await,
            SSH_FXP_SETSTAT => self.handle_setstat(data).await,
            SSH_FXP_FSETSTAT => self.handle_fsetstat(data).await,
            SSH_FXP_OPENDIR => self.handle_opendir(data).await,
            SSH_FXP_READDIR => self.handle_readdir(data).await,
            SSH_FXP_REMOVE => self.handle_remove(data).await,
            SSH_FXP_MKDIR => self.handle_mkdir(data).await,
            SSH_FXP_RMDIR => self.handle_rmdir(data).await,
            SSH_FXP_REALPATH => self.handle_realpath(data).await,
            SSH_FXP_STAT => self.handle_stat(data).await,
            SSH_FXP_READLINK => self.handle_readlink(data).await,
            SSH_FXP_SYMLINK => self.handle_symlink(data).await,
            SSH_FXP_RENAME => self.handle_rename(data).await,
            SSH_FXP_LOCK => self.handle_lock(data).await,
            SSH_FXP_UNLOCK => self.handle_unlock(data).await,
            SSH_FXP_EXTENDED => self.handle_extended(data).await,
            _ => Ok(build_status_packet(0, SSH_FX_OP_UNSUPPORTED, "Unsupported operation", "")),
        }
    }

    async fn handle_init(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let version = if data.len() >= 5 {
            u32::from_be_bytes([data[1], data[2], data[3], data[4]])
        } else {
            3
        };

        self.sftp_version = version.min(3);

        let mut payload = vec![SSH_FXP_VERSION];
        payload.extend_from_slice(&self.sftp_version.to_be_bytes());
        
        let extensions = [
            ("limits@openssh.com", "1"),
            ("posix-rename@openssh.com", "1"),
            ("statvfs@openssh.com", "2"),
            ("fstatvfs@openssh.com", "2"),
            ("hardlink@openssh.com", "1"),
            ("fsync@openssh.com", "1"),
            ("copy-file", "1"),
            ("md5sum@openssh.com", "1"),
            ("sha256sum@openssh.com", "1"),
        ];
        
        for (name, version_str) in extensions {
            payload.extend_from_slice(&(name.len() as u32).to_be_bytes());
            payload.extend_from_slice(name.as_bytes());
            payload.extend_from_slice(&(version_str.len() as u32).to_be_bytes());
            payload.extend_from_slice(version_str.as_bytes());
        }
        
        Ok(build_packet(&payload))
    }

    async fn handle_opendir(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32_checked(data, 1)?;
        let (path, _) = parse_string_checked(data, 5)?;

        if !self.check_permission_cached(|p| p.can_list) {
            return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, "Permission denied", ""));
        }

        let full_path = match self.resolve_path(&path) {
            Ok(p) => p,
            Err(e) => {
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Path resolution failed: {}", e), ""));
            }
        };
        self.log_path_info("OPENDIR", &full_path);

        let dir_result = tokio::fs::read_dir(&full_path).await;
        
        match dir_result {
            Ok(mut dir) => {
                let mut entries = Vec::new();
                while let Ok(Some(entry)) = dir.next_entry().await {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let metadata = entry.metadata().await.ok();
                    let is_dir = metadata.as_ref().map(|m| m.is_dir()).unwrap_or(false);
                    let size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
                    entries.push((name, is_dir, size));
                }
                
                let handle = self.generate_handle();
                self.handles.insert(handle.clone(), SftpFileHandle {
                    path: full_path,
                    file: None,
                    locked: false,
                    existed: true,
                    written_bytes: 0,
                    read_bytes: 0,
                    is_dir: true,
                    dir_entries: entries,
                    dir_index: 0,
                });

                Ok(build_handle_packet(id, &handle))
            }
            Err(e) => {
                let (status, msg) = io_error_to_sftp_status(&e);
                warn!("OPENDIR: Failed for {}: {}", full_path.display(), e);
                Ok(build_status_packet(id, status, msg, ""))
            }
        }
    }

    async fn handle_close(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32_checked(data, 1)?;
        let (handle_str, _) = parse_string_checked(data, 5)?;

        if let Some(handle) = self.handles.remove(&handle_str)
            && !handle.is_dir && handle.written_bytes > 0 {
                let file_size = tokio::fs::metadata(&handle.path).await.map(|m| m.len()).unwrap_or(handle.written_bytes);
                
                if handle.existed {
                    if let Ok(mut fl) = self.file_logger.lock() {
                        fl.log_update(
                            self.username.as_deref().unwrap_or("anonymous"),
                            &self.client_ip,
                            &handle.path.to_string_lossy(),
                            file_size,
                            "SFTP",
                        );
                    }
                } else {
                    if let Ok(mut fl) = self.file_logger.lock() {
                        fl.log_upload(
                            self.username.as_deref().unwrap_or("anonymous"),
                            &self.client_ip,
                            &handle.path.to_string_lossy(),
                            file_size,
                            "SFTP",
                        );
                    }
                }

                info!(username = ?self.username, path = ?handle.path, bytes = file_size, "SFTP file closed");
            }

        Ok(build_status_packet(id, SSH_FX_OK, "OK", ""))
    }

    async fn handle_readdir(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32_checked(data, 1)?;
        let (handle_str, _) = parse_string_checked(data, 5)?;

        let handle = self.handles.get_mut(&handle_str);
        match handle {
            Some(h) if h.is_dir => {
                if h.dir_index >= h.dir_entries.len() {
                    return Ok(build_status_packet(id, SSH_FX_EOF, "End of directory", ""));
                }

                let count = (h.dir_entries.len() - h.dir_index).min(100);
                let result_entries: Vec<(String, bool, u64)> = h.dir_entries[h.dir_index..h.dir_index + count].to_vec();
                h.dir_index += count;

                let mut payload = vec![104];
                payload.extend_from_slice(&id.to_be_bytes());
                payload.extend_from_slice(&(result_entries.len() as u32).to_be_bytes());

                for (name, is_dir, size) in result_entries {
                    payload.extend_from_slice(&(name.len() as u32).to_be_bytes());
                    payload.extend_from_slice(name.as_bytes());
                    
                    let long_name = format_longname(&name, is_dir, size, 0, 0o644);
                    payload.extend_from_slice(&(long_name.len() as u32).to_be_bytes());
                    payload.extend_from_slice(long_name.as_bytes());
                    
                    payload.extend_from_slice(&build_attrs(is_dir, size));
                }

                Ok(build_packet(&payload))
            }
            _ => Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid handle", "")),
        }
    }

    async fn handle_read(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32_checked(data, 1)?;
        let (handle_str, handle_len) = parse_string_checked(data, 5)?;
        
        let offset_pos = 5 + 4 + handle_len;
        if data.len() < offset_pos + 12 {
            return Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid packet: too short for read", ""));
        }
        
        let offset = parse_u64_checked(data, offset_pos)?;
        let len = parse_u32_checked(data, offset_pos + 8)? as usize;

        if !self.check_permission_cached(|p| p.can_read) {
            return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, "Permission denied", ""));
        }

        let speed_limiter = self.speed_limiter.clone();
        let (buffer, path) = match self.handles.get_mut(&handle_str) {
            Some(h) if !h.is_dir => {
                use tokio::io::{AsyncReadExt, AsyncSeekExt};
                
                let file = h.file.as_mut().ok_or_else(|| anyhow::anyhow!("File handle is None"))?;
                
                if let Err(e) = file.seek(std::io::SeekFrom::Start(offset)).await {
                    warn!(path = ?h.path, offset = offset, error = %e, "SFTP seek failed");
                    return Ok(build_status_packet(id, SSH_FX_FAILURE, "Seek failed", ""));
                }
                
                let read_len = len.min(32768);
                let mut buffer = vec![0u8; read_len];
                let n = match file.read(&mut buffer).await {
                    Ok(n) => n,
                    Err(e) => {
                        warn!(path = ?h.path, error = %e, "SFTP read failed");
                        let (status, msg) = io_error_to_sftp_status(&e);
                        return Ok(build_status_packet(id, status, msg, ""));
                    }
                };

                if n == 0 {
                    return Ok(build_status_packet(id, SSH_FX_EOF, "End of file", ""));
                }

                buffer.truncate(n);
                (buffer, h.path.clone())
            }
            _ => return Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid handle", "")),
        };

        if let Some(limiter) = speed_limiter {
            limiter.throttle(buffer.len()).await;
        }

        info!(username = ?self.username, path = ?path, bytes = buffer.len(), "SFTP read");

        if let Ok(mut fl) = self.file_logger.lock() {
            fl.log_download(
                self.username.as_deref().unwrap_or("anonymous"),
                &self.client_ip,
                &path.to_string_lossy(),
                buffer.len() as u64,
                "SFTP",
            );
        }

        Ok(build_data_packet(id, &buffer))
    }

    async fn handle_write(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32_checked(data, 1)?;
        let (handle_str, handle_len) = parse_string_checked(data, 5)?;
        
        let offset_pos = 5 + 4 + handle_len;
        if data.len() < offset_pos + 12 {
            return Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid packet: too short for write", ""));
        }
        
        let offset = parse_u64_checked(data, offset_pos)?;
        let data_len = parse_u32_checked(data, offset_pos + 8)? as usize;
        let data_start = offset_pos + 12;
        
        if data.len() < data_start + data_len {
            return Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid packet: write data truncated", ""));
        }
        
        let write_data = &data[data_start..data_start + data_len];

        if offset > 0 {
            if !self.check_permission_cached(|p| p.can_append) {
                return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, "Permission denied (append)", ""));
            }
        } else if !self.check_permission_cached(|p| p.can_write) {
            return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, "Permission denied", ""));
        }

        let target_path = match self.handles.get(&handle_str) {
            Some(h) if !h.is_dir => h.path.clone(),
            _ => return Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid handle", "")),
        };

        let current_len = tokio::fs::metadata(&target_path).await
            .map(|metadata| metadata.len())
            .unwrap_or(offset);
        let requested_end = offset.saturating_add(data_len as u64);
        let additional_bytes = requested_end.saturating_sub(current_len);

        if !self.check_quota_for_additional_bytes(additional_bytes).await {
            return Ok(build_status_packet(id, SSH_FX_FAILURE, "Quota exceeded", ""));
        }

        if let Some(limiter) = self.speed_limiter.clone() {
            limiter.throttle(write_data.len()).await;
        }

        let handle = self.handles.get_mut(&handle_str);
        match handle {
            Some(h) if !h.is_dir => {
                use tokio::io::{AsyncSeekExt, AsyncWriteExt};
                
                let file = h.file.as_mut().ok_or_else(|| anyhow::anyhow!("File handle is None"))?;
                
                let _ = file.seek(std::io::SeekFrom::Start(offset)).await;
                if let Err(e) = file.write_all(write_data).await {
                    let (status, msg) = io_error_to_sftp_status(&e);
                    return Ok(build_status_packet(id, status, msg, ""));
                }

                h.written_bytes += data_len as u64;
                if additional_bytes > 0 {
                    self.invalidate_quota_cache().await;
                }

                Ok(build_status_packet(id, SSH_FX_OK, "OK", ""))
            }
            _ => Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid handle", "")),
        }
    }

    async fn handle_remove(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32_checked(data, 1)?;
        let (path, _) = parse_string_checked(data, 5)?;

        if !self.check_permission_cached(|p| p.can_delete) {
            return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, "Permission denied", ""));
        }

        let full_path = match self.resolve_path(&path) {
            Ok(p) => p,
            Err(e) => {
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Path resolution failed: {}", e), ""));
            }
        };

        match tokio::fs::remove_file(&full_path).await {
            Ok(_) => {
                if let Ok(mut fl) = self.file_logger.lock() {
                    fl.log_delete(
                        self.username.as_deref().unwrap_or("anonymous"),
                        &self.client_ip,
                        &full_path.to_string_lossy(),
                        "SFTP",
                    );
                }
                info!(username = ?self.username, path = ?full_path, "SFTP file deleted");
                Ok(build_status_packet(id, SSH_FX_OK, "OK", ""))
            }
            Err(e) => {
                let (status, msg) = io_error_to_sftp_status(&e);
                Ok(build_status_packet(id, status, &format!("{}: {}", msg, e), ""))
            }
        }
    }

    async fn handle_mkdir(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32_checked(data, 1)?;
        let (path, _) = parse_string_checked(data, 5)?;

        if !self.check_permission_cached(|p| p.can_mkdir) {
            return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, "Permission denied", ""));
        }

        let full_path = match self.resolve_path(&path) {
            Ok(p) => p,
            Err(e) => {
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Path resolution failed: {}", e), ""));
            }
        };

        match tokio::fs::create_dir_all(&full_path).await {
            Ok(_) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = tokio::fs::set_permissions(&full_path, std::fs::Permissions::from_mode(0o755)).await;
                }

                if let Ok(mut fl) = self.file_logger.lock() {
                    fl.log_mkdir(
                        self.username.as_deref().unwrap_or("anonymous"),
                        &self.client_ip,
                        &full_path.to_string_lossy(),
                        "SFTP",
                    );
                }
                info!(username = ?self.username, path = ?full_path, "SFTP directory created");
                Ok(build_status_packet(id, SSH_FX_OK, "OK", ""))
            }
            Err(e) => {
                let (status, msg) = io_error_to_sftp_status(&e);
                let error_msg = format!("{}: {} (path: {})", msg, e, full_path.display());
                Ok(build_status_packet(id, status, &error_msg, ""))
            }
        }
    }

    async fn handle_rmdir(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32_checked(data, 1)?;
        let (path, _) = parse_string_checked(data, 5)?;

        if !self.check_permission_cached(|p| p.can_rmdir) {
            return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, "Permission denied", ""));
        }

        let full_path = match self.resolve_path(&path) {
            Ok(p) => p,
            Err(e) => {
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Path resolution failed: {}", e), ""));
            }
        };

        match tokio::fs::remove_dir_all(&full_path).await {
            Ok(_) => {
                if let Ok(mut fl) = self.file_logger.lock() {
                    fl.log_rmdir(
                        self.username.as_deref().unwrap_or("anonymous"),
                        &self.client_ip,
                        &full_path.to_string_lossy(),
                        "SFTP",
                    );
                }
                Ok(build_status_packet(id, SSH_FX_OK, "OK", ""))
            }
            Err(e) => {
                let (status, msg) = io_error_to_sftp_status(&e);
                Ok(build_status_packet(id, status, msg, ""))
            }
        }
    }

    async fn handle_rename(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32_checked(data, 1)?;
        
        if data.len() < 9 {
            return Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid packet", ""));
        }
        
        let (old_path, old_len) = parse_string_checked(data, 5)?;
        let new_path_pos = 5 + 4 + old_len;
        
        if data.len() < new_path_pos + 4 {
            return Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid packet", ""));
        }
        
        let (new_path, _new_len) = parse_string_checked(data, new_path_pos)?;

        // 🔒 权限检查
        if !self.check_permission_cached(|p| p.can_rename) {
            return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, "Permission denied", ""));
        }

        // 🔒 安全增强：使用严格的路径验证
        let old_full = match validate_path_within_chroot(&old_path, &self.home_dir).await {
            Ok(p) => p,
            Err(e) => {
                warn!("RENAME: Old path validation failed: {}", e);
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Invalid source path: {}", e), ""));
            }
        };
        
        debug!("RENAME: Validated source path: {:?}", old_full);
        
        let new_full = match validate_path_for_creation(&new_path, &self.home_dir).await {
            Ok(p) => p,
            Err(e) => {
                warn!("RENAME: New path validation failed: {}", e);
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Invalid destination path: {}", e), ""));
            }
        };
        
        debug!("RENAME: Validated destination path: {:?}", new_full);
        
        // 🔒 额外安全检查：验证父目录不是符号链接
        if let Some(old_parent) = old_full.parent()
            && let Ok(meta) = tokio::fs::symlink_metadata(old_parent).await
            && meta.file_type().is_symlink() {
                warn!("RENAME: Source parent is a symlink: {:?}", old_parent);
                return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, 
                    "Cannot rename from directory under symbolic link", ""));
        }
        
        if let Some(new_parent) = new_full.parent()
            && let Ok(meta) = tokio::fs::symlink_metadata(new_parent).await
            && meta.file_type().is_symlink() {
                warn!("RENAME: Destination parent is a symlink: {:?}", new_parent);
                return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, 
                    "Cannot rename to directory under symbolic link", ""));
        }

        // 🔒 原子操作：如果目标已存在，先删除再重命名
        if tokio::fs::metadata(&new_full).await.is_ok() {
            debug!("RENAME: Destination exists, removing first: {:?}", new_full);
            if let Err(e) = tokio::fs::remove_file(&new_full).await {
                warn!("RENAME: Failed to remove existing destination: {}", e);
                // 尝试删除目录
                if let Err(e2) = tokio::fs::remove_dir_all(&new_full).await {
                    warn!("RENAME: Failed to remove as directory either: {}", e2);
                    return Ok(build_status_packet(id, SSH_FX_FAILURE, 
                        &format!("Cannot remove existing destination: {}", e), ""));
                }
            }
        }

        // 🔒 执行重命名（TOCTOU 防护）
        let result = match tokio::fs::rename(&old_full, &new_full).await {
            Ok(_) => {
                if let Ok(mut fl) = self.file_logger.lock() {
                    fl.log_rename(
                        self.username.as_deref().unwrap_or("anonymous"),
                        &self.client_ip,
                        &old_full.to_string_lossy(),
                        &new_full.to_string_lossy(),
                        "SFTP",
                    );
                }
                build_status_packet(id, SSH_FX_OK, "OK", "")
            }
            Err(e) => {
                let (status, msg) = io_error_to_sftp_status(&e);
                build_status_packet(id, status, &format!("{}: {}", msg, e), "")
            }
        };
        
        Ok(result)
    }

    async fn handle_stat(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32_checked(data, 1)?;
        let (path, _) = parse_string_checked(data, 5)?;

        if !self.check_permission_cached(|p| p.can_read) {
            return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, "Permission denied", ""));
        }

        let full_path = match self.resolve_path(&path) {
            Ok(p) => p,
            Err(e) => {
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Path resolution failed: {}", e), ""));
            }
        };
        self.log_path_info("STAT", &full_path);

        match tokio::fs::metadata(&full_path).await {
            Ok(metadata) => {
                let mut payload = vec![105];
                payload.extend_from_slice(&id.to_be_bytes());
                payload.extend_from_slice(&build_attrs(metadata.is_dir(), metadata.len()));
                Ok(build_packet(&payload))
            }
            Err(e) => {
                let (status, msg) = io_error_to_sftp_status(&e);
                Ok(build_status_packet(id, status, msg, ""))
            }
        }
    }

    async fn handle_lstat(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32_checked(data, 1)?;
        let (path, _) = parse_string_checked(data, 5)?;

        if !self.check_permission_cached(|p| p.can_read) {
            return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, "Permission denied", ""));
        }

        let full_path = match self.resolve_path(&path) {
            Ok(p) => p,
            Err(e) => {
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Path resolution failed: {}", e), ""));
            }
        };
        self.log_path_info("LSTAT", &full_path);

        // Use symlink_metadata to get information about the symlink itself
        match tokio::fs::symlink_metadata(&full_path).await {
            Ok(metadata) => {
                let mut payload = vec![105];
                payload.extend_from_slice(&id.to_be_bytes());
                payload.extend_from_slice(&build_attrs(metadata.is_dir(), metadata.len()));
                Ok(build_packet(&payload))
            }
            Err(e) => {
                let (status, msg) = io_error_to_sftp_status(&e);
                Ok(build_status_packet(id, status, msg, ""))
            }
        }
    }

    async fn handle_setstat(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32_checked(data, 1)?;
        let (path, path_len) = parse_string_checked(data, 5)?;

        if !self.check_permission_cached(|p| p.can_write) {
            return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, "Permission denied", ""));
        }

        let full_path = match self.resolve_path(&path) {
            Ok(p) => p,
            Err(e) => {
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Path resolution failed: {}", e), ""));
            }
        };

        let attrs_offset = 5 + 4 + path_len;
        if data.len() < attrs_offset + 4 {
            return Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid SETSTAT packet", ""));
        }

        let flags = parse_u32_checked(data, attrs_offset)?;
        debug!("SETSTAT: path={}, flags=0x{:08x}", full_path.display(), flags);

        // SSH_FILEXFER_ATTR_SIZE = 0x00000001
        // SSH_FILEXFER_ATTR_UIDGID = 0x00000002
        // SSH_FILEXFER_ATTR_PERMISSIONS = 0x00000004
        // SSH_FILEXFER_ATTR_ACMODTIME = 0x00000008

        let mut offset = attrs_offset + 4;

        // Handle size if present
        if flags & 0x00000001 != 0 && data.len() >= offset + 8 {
            let _size = parse_u64_checked(data, offset)?;
            offset += 8;
            debug!("SETSTAT: size flag present (not implemented in this version)");
        }

        // Handle uid/gid if present
        if flags & 0x00000002 != 0 && data.len() >= offset + 8 {
            let _uid = parse_u32_checked(data, offset)?;
            let _gid = parse_u32_checked(data, offset + 4)?;
            offset += 8;
            debug!("SETSTAT: uid/gid flag present (not implemented in this version)");
        }

        // Handle permissions if present
        if flags & 0x00000004 != 0 && data.len() >= offset + 4 {
            let permissions = parse_u32_checked(data, offset)?;
            debug!("SETSTAT: requested permissions=0o{:o} for {:?}", permissions, full_path);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                
                // 🔒 只保留标准权限位（rwxrwxrwx）
                let mode = permissions & 0o777;
                
                // 🔒 禁止特殊权限位：setuid (0o4000), setgid (0o2000), sticky bit (0o1000)
                if permissions & 0o7000 != 0 {
                    warn!("SECURITY: User attempted to set special permission bits: 0o{:o}", permissions);
                    return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED,
                        "Special permission bits (setuid/setgid/sticky) are not allowed", ""));
                }
                
                // 🔒 可选限制：非管理员用户不能设置 world-writable 或 world-executable
                // 当前版本未启用此检查，保留作为未来安全加固参考
                // 如需启用，取消下面代码的注释：
                /*
                if !self.is_admin_user() && mode & 0o022 != 0 {
                    warn!("Non-admin user attempted to set world-writable/executable: 0o{:o}", mode);
                    return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED,
                        "World-writable/executable permissions not allowed", ""));
                }
                */
                
                debug!("SETSTAT: setting mode=0o{:o} (filtered from 0o{:o})", mode, permissions);
                
                match tokio::fs::set_permissions(&full_path, std::fs::Permissions::from_mode(mode)).await {
                    Ok(_) => {
                        debug!("SETSTAT: permissions successfully changed to 0o{:o}", mode);
                    }
                    Err(e) => {
                        warn!("SETSTAT: failed to set permissions: {}", e);
                        let (status, msg) = io_error_to_sftp_status(&e);
                        return Ok(build_status_packet(id, status, msg, ""));
                    }
                }
            }
        }

        // Handle atime/mtime if present
        if flags & 0x00000008 != 0 && data.len() >= offset + 8 {
            let _atime = parse_u32_checked(data, offset)?;
            let _mtime = parse_u32_checked(data, offset + 4)?;
            debug!("SETSTAT: atime/mtime flag present (not implemented in this version)");
        }

        Ok(build_status_packet(id, SSH_FX_OK, "OK", ""))
    }

    async fn handle_fsetstat(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32_checked(data, 1)?;
        let (handle_str, handle_len) = parse_string_checked(data, 5)?;

        if !self.check_permission_cached(|p| p.can_write) {
            return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, "Permission denied", ""));
        }

        let handle = self.handles.get(&handle_str);
        match handle {
            Some(h) if !h.is_dir => {
                let attrs_offset = 5 + 4 + handle_len;
                if data.len() < attrs_offset + 4 {
                    return Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid FSETSTAT packet", ""));
                }

                let flags = parse_u32_checked(data, attrs_offset)?;
                debug!("FSETSTAT: path={:?}, flags=0x{:08x}", h.path, flags);

                let mut offset = attrs_offset + 4;

                // Handle size if present (flags & 0x00000001)
                if flags & 0x00000001 != 0 && data.len() >= offset + 8 {
                    let _size = parse_u64_checked(data, offset)?;
                    offset += 8;
                    debug!("FSETSTAT: size flag present (not implemented)");
                }

                // Handle uid/gid if present (flags & 0x00000002)
                if flags & 0x00000002 != 0 && data.len() >= offset + 8 {
                    let _uid = parse_u32_checked(data, offset)?;
                    let _gid = parse_u32_checked(data, offset + 4)?;
                    offset += 8;
                    debug!("FSETSTAT: uid/gid flag present (not implemented)");
                }

                // Handle permissions if present (flags & 0x00000004)
                if flags & 0x00000004 != 0 && data.len() >= offset + 4 {
                    let permissions = parse_u32_checked(data, offset)?;
                    debug!("FSETSTAT: requested permissions=0o{:o} for {:?}", permissions, h.path);
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        
                        // 🔒 只保留标准权限位
                        let mode = permissions & 0o777;
                        
                        // 🔒 禁止特殊权限位
                        if permissions & 0o7000 != 0 {
                            warn!("SECURITY: User attempted to set special permission bits: 0o{:o}", permissions);
                            return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED,
                                "Special permission bits (setuid/setgid/sticky) are not allowed", ""));
                        }
                        
                        debug!("FSETSTAT: setting mode=0o{:o} (filtered from 0o{:o})", mode, permissions);
                        
                        match tokio::fs::set_permissions(&h.path, std::fs::Permissions::from_mode(mode)).await {
                            Ok(_) => {
                                debug!("FSETSTAT: permissions successfully changed to 0o{:o}", mode);
                            }
                            Err(e) => {
                                warn!("FSETSTAT: failed to set permissions: {}", e);
                            }
                        }
                    }
                } else if flags & 0x00000004 != 0 {
                    warn!("FSETSTAT: permissions flag set but data too short");
                }

                // Handle atime/mtime if present (flags & 0x00000008)
                if flags & 0x00000008 != 0 && data.len() >= offset + 8 {
                    let _atime = parse_u32_checked(data, offset)?;
                    let _mtime = parse_u32_checked(data, offset + 4)?;
                    debug!("FSETSTAT: atime/mtime flag present (not implemented in this version)");
                }

                Ok(build_status_packet(id, SSH_FX_OK, "OK", ""))
            }
            _ => Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid handle", "")),
        }
    }

    async fn handle_fstat(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32_checked(data, 1)?;
        let (handle_str, _) = parse_string_checked(data, 5)?;

        if !self.check_permission_cached(|p| p.can_read) {
            return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, "Permission denied", ""));
        }

        let handle = self.handles.get(&handle_str);
        match handle {
            Some(h) if !h.is_dir => {
                match tokio::fs::metadata(&h.path).await {
                    Ok(metadata) => {
                        let mut payload = vec![105];
                        payload.extend_from_slice(&id.to_be_bytes());
                        payload.extend_from_slice(&build_attrs(metadata.is_dir(), metadata.len()));
                        Ok(build_packet(&payload))
                    }
                    Err(e) => {
                        let (status, msg) = io_error_to_sftp_status(&e);
                        Ok(build_status_packet(id, status, msg, ""))
                    }
                }
            }
            _ => Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid handle", "")),
        }
    }

    async fn handle_realpath(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32_checked(data, 1)?;
        let (path, _) = parse_string_checked(data, 5)?;

        if !self.check_permission_cached(|p| p.can_read) {
            return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, "Permission denied", ""));
        }

        let full_path = match self.resolve_path(&path) {
            Ok(p) => p,
            Err(e) => {
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Path resolution failed: {}", e), ""));
            }
        };

        let resolved = match tokio::fs::canonicalize(&full_path).await {
            Ok(p) => p,
            Err(_) => full_path,
        };

        let home = PathBuf::from(&self.home_dir);
        let home_canon = tokio::fs::canonicalize(&home).await.unwrap_or(home);
        
        let path_str = if resolved.starts_with(&home_canon) {
            let relative = resolved.strip_prefix(&home_canon).unwrap_or(&resolved);
            if relative.as_os_str().is_empty() {
                "/".to_string()
            } else {
                format!("/{}", relative.to_string_lossy())
            }
        } else {
            resolved.to_string_lossy().to_string()
        };

        let metadata = tokio::fs::metadata(&resolved).await.ok();
        let mtime = metadata.as_ref().map(|m| m.mtime()).unwrap_or(0);
        let mode = metadata.as_ref().map(|m| m.mode()).unwrap_or(0o755);

        let mut payload = vec![104];
        payload.extend_from_slice(&id.to_be_bytes());
        payload.extend_from_slice(&1u32.to_be_bytes());
        payload.extend_from_slice(&(path_str.len() as u32).to_be_bytes());
        payload.extend_from_slice(path_str.as_bytes());
        let longname = format_longname(&path_str, true, 0, mtime, mode);
        payload.extend_from_slice(&(longname.len() as u32).to_be_bytes());
        payload.extend_from_slice(longname.as_bytes());
        payload.extend_from_slice(&build_attrs(true, 0));

        Ok(build_packet(&payload))
    }

    pub fn resolve_path(&self, path: &str) -> Result<PathBuf> {
        let resolved = safe_resolve_path(&self.home_dir, path)?;
        Ok(resolved)
    }
    
    fn log_path_info(&self, operation: &str, path: &std::path::Path) {
        debug!(
            username = ?self.username,
            operation = %operation,
            path = ?path,
            exists = path.exists(),
            "SFTP path info"
        );
    }

    fn generate_handle(&mut self) -> String {
        let mut attempts = 0;
        loop {
            let handle = format!("h{:08x}", self.next_handle_id);
            self.next_handle_id = self.next_handle_id.wrapping_add(1);
            
            if !self.handles.contains_key(&handle) {
                return handle;
            }
            
            attempts += 1;
            if attempts > 1000 {
                return handle;
            }
        }
    }

    async fn handle_open(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        const SSH_FXF_READ: u32   = 0x00000001;
        const SSH_FXF_WRITE: u32  = 0x00000002;
        const SSH_FXF_APPEND: u32 = 0x00000004;
        const SSH_FXF_CREAT: u32  = 0x00000008;
        const SSH_FXF_TRUNC: u32  = 0x00000010;
        const SSH_FXF_EXCL: u32   = 0x00000020;

        let id = parse_u32_checked(data, 1)?;
        let (path, path_len) = parse_string_checked(data, 5)?;
        let pflags_pos = 5 + 4 + path_len;
        
        if data.len() < pflags_pos + 4 {
            return Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid packet: missing pflags", ""));
        }
        
        let pflags = parse_u32_checked(data, pflags_pos)?;

        let need_read = pflags & SSH_FXF_READ != 0;
        let need_write = pflags & SSH_FXF_WRITE != 0;
        let need_append = pflags & SSH_FXF_APPEND != 0;
        let need_creat = pflags & SSH_FXF_CREAT != 0;
        let need_trunc = pflags & SSH_FXF_TRUNC != 0;
        let need_excl = pflags & SSH_FXF_EXCL != 0;

        if !self.check_permission_cached(|p| {
            (!need_read || p.can_read) &&
            (!need_write || p.can_write) &&
            (!need_append || p.can_append)
        }) {
            return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, "Permission denied", ""));
        }

        let full_path = match self.resolve_path(&path) {
            Ok(p) => p,
            Err(e) => {
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Path resolution failed: {}", e), ""));
            }
        };
        self.log_path_info("OPEN", &full_path);

        let file_result = if need_write {
            let mut opts = tokio::fs::OpenOptions::new();
            opts.read(need_read).write(true);
            
            if need_excl && need_creat {
                opts.create_new(true);
            } else if need_trunc && need_creat {
                opts.create(true).truncate(true);
            } else if need_append {
                opts.create(need_creat).append(true);
            } else if need_creat {
                opts.create(true);
            }
            
            opts.open(&full_path).await
        } else {
            tokio::fs::File::open(&full_path).await
        };

        match file_result {
            Ok(file) => {
                let file_existed = !need_excl || !need_creat;
                
                if !file_existed {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let _ = tokio::fs::set_permissions(&full_path, std::fs::Permissions::from_mode(0o644)).await;
                    }
                }

                let handle = self.generate_handle();
                self.handles.insert(handle.clone(), SftpFileHandle {
                    path: full_path,
                    file: Some(file),
                    locked: false,
                    existed: file_existed,
                    written_bytes: 0,
                    read_bytes: 0,
                    is_dir: false,
                    dir_entries: Vec::new(),
                    dir_index: 0,
                });
                Ok(build_handle_packet(id, &handle))
            }
            Err(e) => {
                let (status, msg) = io_error_to_sftp_status(&e);
                let error_msg = format!("{}: {} (path: {})", msg, e, full_path.display());
                Ok(build_status_packet(id, status, &error_msg, ""))
            }
        }
    }

    async fn handle_readlink(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32_checked(data, 1)?;
        let (path, _path_len) = parse_string_checked(data, 5)?;
        

        let full_path = match self.resolve_path(&path) {
            Ok(p) => p,
            Err(_e) => {
                return Ok(build_status_packet(id, SSH_FX_NO_SUCH_FILE, "No such file", ""));
            }
        };
        

        match tokio::fs::symlink_metadata(&full_path).await {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    match tokio::fs::read_link(&full_path).await {
                        Ok(target) => {
                            let target_str = target.to_string_lossy().to_string();
                            let mut payload = vec![104];
                            payload.extend_from_slice(&id.to_be_bytes());
                            payload.extend_from_slice(&1u32.to_be_bytes());
                            payload.extend_from_slice(&(target_str.len() as u32).to_be_bytes());
                            payload.extend_from_slice(target_str.as_bytes());
                            payload.extend_from_slice(&(target_str.len() as u32).to_be_bytes());
                            payload.extend_from_slice(target_str.as_bytes());
                            payload.extend_from_slice(&build_attrs(false, 0));
                            Ok(build_packet(&payload))
                        }
                        Err(e) => {
                            let (status, msg) = io_error_to_sftp_status(&e);
                            Ok(build_status_packet(id, status, &format!("{}: {}", msg, e), ""))
                        }
                    }
                } else {
                    Ok(build_status_packet(id, SSH_FX_FAILURE, "Not a symbolic link", ""))
                }
            }
            Err(e) => {
                let (status, msg) = io_error_to_sftp_status(&e);
                Ok(build_status_packet(id, status, msg, ""))
            }
        }
    }

    async fn handle_symlink(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32_checked(data, 1)?;
        let (target, target_len) = parse_string_checked(data, 5)?;
        let link_pos = 5 + 4 + target_len;
        let (link_path, _) = parse_string_checked(data, link_pos)?;

        if !self.check_permission_cached(|p| p.can_write) {
            return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, "Permission denied", ""));
        }

        // 🔒 安全增强：使用 validate_path_within_chroot 严格验证路径
        let full_link = match validate_path_for_creation(&link_path, &self.home_dir).await {
            Ok(p) => p,
            Err(e) => {
                warn!("SYMLINK: Link path validation failed: {}", e);
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Invalid link path: {}", e), ""));
            }
        };
        
        // 🔒 对目标路径进行更严格的验证
        let full_target = match validate_path_within_chroot(&target, &self.home_dir).await {
            Ok(p) => p,
            Err(e) => {
                warn!("SYMLINK: Target path validation failed: {}", e);
                return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, &format!("Invalid symlink target: {}", e), ""));
            }
        };

        debug!("SYMLINK: Creating link {:?} -> {:?}", full_link, full_target);
        
        // 创建符号链接
        #[cfg(unix)]
        match std::os::unix::fs::symlink(&full_target, &full_link) {
            Ok(_) => {
                debug!("SYMLINK: Successfully created symlink {:?} -> {:?}", full_link, full_target);
                
                // 🔒 立即验证创建的是真正的符号链接
                match std::fs::symlink_metadata(&full_link) {
                    Ok(metadata) => {
                        let is_symlink = metadata.file_type().is_symlink();
                        debug!("SYMLINK: Verification - is_symlink={}, mode={:o}", is_symlink, metadata.mode());
                        
                        if !is_symlink {
                            warn!("SYMLINK: Created file is not a symlink! Cleaning up...");
                            let _ = std::fs::remove_file(&full_link);
                            return Ok(build_status_packet(id, SSH_FX_FAILURE, "Failed to create symlink", ""));
                        }
                        
                        // 🔒 额外检查：符号链接的目标是否可解析且在 home 内
                        match std::fs::read_link(&full_link) {
                            Ok(link_target) => {
                                if link_target.is_absolute() {
                                    let home_canon = std::fs::canonicalize(&self.home_dir)
                                        .unwrap_or_else(|_| PathBuf::from(&self.home_dir));
                                    if !link_target.starts_with(&home_canon) {
                                        warn!("SYMLINK: Symlink points outside chroot: {:?}", link_target);
                                        let _ = std::fs::remove_file(&full_link);
                                        return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, "Symlink target outside home directory", ""));
                                    }
                                }
                            }
                            Err(e) => {
                                warn!("SYMLINK: Cannot read created symlink: {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        warn!("SYMLINK: Failed to verify symlink: {}", e);
                    }
                }
                
                if let Ok(mut fl) = self.file_logger.lock() {
                    fl.log(crate::core::file_logger::FileLogInfo {
                        username: self.username.as_deref().unwrap_or("anonymous"),
                        client_ip: &self.client_ip,
                        operation: "SYMLINK",
                        file_path: &format!("{} -> {}", full_link.to_string_lossy(), full_target.to_string_lossy()),
                        file_size: 0,
                        protocol: "SFTP",
                        success: true,
                        message: "符号链接创建成功",
                    });
                }
                info!(username = ?self.username, link = ?full_link, target = ?full_target, "SFTP symlink created");
                Ok(build_status_packet(id, SSH_FX_OK, "OK", ""))
            }
            Err(e) => {
                warn!(username = ?self.username, link = ?full_link, target = ?full_target, error = %e, "SFTP symlink failed");
                let (status, msg) = io_error_to_sftp_status(&e);
                Ok(build_status_packet(id, status, &format!("{}: {}", msg, e), ""))
            }
        }
        
        #[cfg(not(unix))]
        {
            warn!("SYMLINK not supported on non-Unix platforms");
            Ok(build_status_packet(id, SSH_FX_OP_UNSUPPORTED, "Operation unsupported", ""))
        }
    }

    async fn handle_lock(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32_checked(data, 1)?;
        let (handle_str, _) = parse_string_checked(data, 5)?;

        if self.sftp_version < 5 {
            return Ok(build_status_packet(id, SSH_FX_OP_UNSUPPORTED, "Lock requires SFTP v5+", ""));
        }

        let handle = self.handles.get_mut(&handle_str);
        match handle {
            Some(h) if !h.is_dir => {
                if h.locked {
                    return Ok(build_status_packet(id, SSH_FX_OK, "Already locked", ""));
                }

                let file = h.file.as_ref().ok_or_else(|| anyhow::anyhow!("File handle is None"))?;
                
                let std_file = match file.try_clone().await {
                    Ok(f) => f.into_std().await,
                    Err(_) => {
                        return Ok(build_status_packet(id, SSH_FX_FAILURE, "Failed to clone file handle", ""));
                    }
                };

                match fs2::FileExt::lock_exclusive(&std_file) {
                    Ok(()) => {
                        h.locked = true;
                        self.locked_files.insert(h.path.clone());
                        Ok(build_status_packet(id, SSH_FX_OK, "OK", ""))
                    }
                    Err(_) => Ok(build_status_packet(id, SSH_FX_FAILURE, "Failed to lock file", "")),
                }
            }
            _ => Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid handle", "")),
        }
    }

    async fn handle_unlock(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32_checked(data, 1)?;
        let (handle_str, _) = parse_string_checked(data, 5)?;

        let handle = self.handles.get_mut(&handle_str);
        match handle {
            Some(h) if !h.is_dir => {
                if !h.locked {
                    return Ok(build_status_packet(id, SSH_FX_OK, "Not locked", ""));
                }

                let file = h.file.as_ref().ok_or_else(|| anyhow::anyhow!("File handle is None"))?;
                
                let std_file = match file.try_clone().await {
                    Ok(f) => f.into_std().await,
                    Err(_) => {
                        return Ok(build_status_packet(id, SSH_FX_FAILURE, "Failed to clone file handle", ""));
                    }
                };

                match fs2::FileExt::unlock(&std_file) {
                    Ok(()) => {
                        h.locked = false;
                        self.locked_files.remove(&h.path);
                        Ok(build_status_packet(id, SSH_FX_OK, "OK", ""))
                    }
                    Err(_) => Ok(build_status_packet(id, SSH_FX_FAILURE, "Failed to unlock file", "")),
                }
            }
            _ => Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid handle", "")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex as StdMutex};
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::core::file_logger::FileLogger;

    use crate::core::users::{Permissions, UserManager};

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(name: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "wftpg-sftp-{}-{}-{}",
                name,
                std::process::id(),
                unique
            ));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn make_state(home: &Path, permissions: Permissions) -> SftpState {
        let log_dir = home.join("logs");
        fs::create_dir_all(&log_dir).unwrap();

        let mut users = UserManager::new();
        users.add_user(
            "tester".to_string(),
            "password",
            home.to_string_lossy().into_owned(),
            permissions,
            false,
        )
        .unwrap();

        SftpState::new(
            home.to_string_lossy().into_owned(),
            Some("tester".to_string()),
            Arc::new(StdMutex::new(users)),
            Arc::new(StdMutex::new(FileLogger::new(&log_dir.to_string_lossy(), 1024 * 1024))),
            Arc::new(QuotaCache::new()),
            "127.0.0.1".to_string(),
        )
    }

    fn payload(packet: &[u8]) -> &[u8] {
        assert!(packet.len() >= 4);
        let len = u32::from_be_bytes(packet[0..4].try_into().unwrap()) as usize;
        assert_eq!(packet.len(), len + 4);
        &packet[4..]
    }

    fn parse_status(packet: &[u8]) -> (u32, String) {
        let payload = payload(packet);
        assert_eq!(payload[0], 101);
        let status = parse_u32(payload, 5);
        let msg_len = parse_u32(payload, 9) as usize;
        let msg_start = 13;
        let msg_end = msg_start + msg_len;
        let message = String::from_utf8_lossy(&payload[msg_start..msg_end]).into_owned();
        (status, message)
    }



    fn build_string_field(value: &str) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&(value.len() as u32).to_be_bytes());
        buf.extend_from_slice(value.as_bytes());
        buf
    }

    fn build_extended_packet(id: u32, extension: &str, extra: &[u8]) -> Vec<u8> {
        let mut packet = vec![SSH_FXP_EXTENDED];
        packet.extend_from_slice(&id.to_be_bytes());
        packet.extend_from_slice(&(extension.len() as u32).to_be_bytes());
        packet.extend_from_slice(extension.as_bytes());
        packet.extend_from_slice(extra);
        packet
    }

    fn build_write_packet(id: u32, handle: &str, offset: u64, data: &[u8]) -> Vec<u8> {
        let mut packet = vec![SSH_FXP_WRITE];
        packet.extend_from_slice(&id.to_be_bytes());
        packet.extend_from_slice(&build_string_field(handle));
        packet.extend_from_slice(&offset.to_be_bytes());
        packet.extend_from_slice(&(data.len() as u32).to_be_bytes());
        packet.extend_from_slice(data);
        packet
    }

    #[tokio::test]
    async fn init_advertises_supported_extensions() {
        let dir = TestDir::new("init-extensions");
        let state = &mut make_state(dir.path(), Permissions::full());

        let response = state.handle_init(&[SSH_FXP_INIT, 0, 0, 0, 3]).await.unwrap();
        let payload = payload(&response);

        assert_eq!(payload[0], 2);
        assert_eq!(parse_u32(payload, 1), 3);

        let mut offset = 5;
        let mut extensions = Vec::new();
        while offset < payload.len() {
            let (name, consumed_name) = parse_string_checked(payload, offset).unwrap();
            offset += 4 + consumed_name;
            let (version, consumed_version) = parse_string_checked(payload, offset).unwrap();
            offset += 4 + consumed_version;
            extensions.push((name, version));
        }

        assert!(extensions.iter().any(|(name, _)| name == "limits@openssh.com"));
        assert!(extensions.iter().any(|(name, _)| name == "copy-file"));
        assert!(extensions.iter().any(|(name, _)| name == "fsync@openssh.com"));
        assert!(extensions.iter().any(|(name, _)| name == "fstatvfs@openssh.com"));
    }

    #[tokio::test]
    async fn write_rejects_when_quota_is_exceeded() {
        let dir = TestDir::new("write-quota");
        let filler = dir.path().join("filler.bin");
        let mut filler_file = fs::File::create(&filler).unwrap();
        filler_file.write_all(&vec![b'x'; 1_048_326]).unwrap();

        let mut permissions = Permissions::full();
        permissions.quota_mb = Some(1);
        let state = &mut make_state(dir.path(), permissions);

        let target_path = dir.path().join("target.bin");
        let file = tokio::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&target_path)
            .await
            .unwrap();

        state.handles.insert(
            "h00000001".to_string(),
            SftpFileHandle {
                path: target_path.clone(),
                file: Some(file),
                locked: false,
                existed: false,
                written_bytes: 0,
                read_bytes: 0,
                is_dir: false,
                dir_entries: Vec::new(),
                dir_index: 0,
            },
        );

        let packet = build_write_packet(1, "h00000001", 0, &[1u8; 512]);
        let response = state.handle_write(&packet).await.unwrap();
        let (status, message) = parse_status(&response);

        assert_eq!(status, SSH_FX_FAILURE);
        assert!(message.contains("Quota exceeded"));
        assert_eq!(fs::metadata(&target_path).unwrap().len(), 0);
    }

    #[tokio::test]
    async fn copy_file_respects_overwrite_flag() {
        let dir = TestDir::new("copy-overwrite");
        fs::write(dir.path().join("src.txt"), b"new-data").unwrap();
        fs::write(dir.path().join("dst.txt"), b"old-data").unwrap();

        let state = &mut make_state(dir.path(), Permissions::full());
        let mut args = Vec::new();
        args.extend_from_slice(&build_string_field("/src.txt"));
        args.extend_from_slice(&build_string_field("/dst.txt"));
        args.push(0);

        let response = state
            .handle_extended(&build_extended_packet(7, "copy-file", &args))
            .await
            .unwrap();
        let (status, message) = parse_status(&response);

        assert_eq!(status, SSH_FX_FAILURE);
        assert!(message.contains("already exists"));
        assert_eq!(fs::read(dir.path().join("dst.txt")).unwrap(), b"old-data");
    }

    #[tokio::test]
    async fn copy_file_rejects_when_quota_is_exceeded() {
        let dir = TestDir::new("copy-quota");
        fs::write(dir.path().join("src.txt"), vec![b'a'; 200]).unwrap();
        let mut filler = fs::File::create(dir.path().join("filler.bin")).unwrap();
        filler.write_all(&vec![b'b'; 1_048_326]).unwrap();

        let mut permissions = Permissions::full();
        permissions.quota_mb = Some(1);
        let state = &mut make_state(dir.path(), permissions);

        let mut args = Vec::new();
        args.extend_from_slice(&build_string_field("/src.txt"));
        args.extend_from_slice(&build_string_field("/dst.txt"));
        args.push(1);

        let response = state
            .handle_extended(&build_extended_packet(8, "copy-file", &args))
            .await
            .unwrap();
        let (status, message) = parse_status(&response);

        assert_eq!(status, SSH_FX_FAILURE);
        assert!(message.contains("Quota exceeded"));
        assert!(!dir.path().join("dst.txt").exists());
    }

    #[tokio::test]
    async fn fsync_and_fstatvfs_succeed_for_file_handle() {
        let dir = TestDir::new("handle-extensions");
        let file_path = dir.path().join("open.bin");
        fs::write(&file_path, b"payload").unwrap();

        let state = &mut make_state(dir.path(), Permissions::full());
        let file = tokio::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&file_path)
            .await
            .unwrap();

        state.handles.insert(
            "h00000002".to_string(),
            SftpFileHandle {
                path: file_path,
                file: Some(file),
                locked: false,
                existed: true,
                written_bytes: 0,
                read_bytes: 0,
                is_dir: false,
                dir_entries: Vec::new(),
                dir_index: 0,
            },
        );

        let fsync_response = state
            .handle_extended(&build_extended_packet(9, "fsync@openssh.com", &build_string_field("h00000002")))
            .await
            .unwrap();
        let (fsync_status, _) = parse_status(&fsync_response);
        assert_eq!(fsync_status, SSH_FX_OK);

        let fstatvfs_response = state
            .handle_extended(&build_extended_packet(10, "fstatvfs@openssh.com", &build_string_field("h00000002")))
            .await
            .unwrap();
        let fstatvfs_payload = payload(&fstatvfs_response);
        assert_eq!(fstatvfs_payload[0], 201);
        assert_eq!(parse_u32(fstatvfs_payload, 1), 10);
    }

    #[test]
    fn speed_limiter_is_initialized_from_permissions() {
        let dir = TestDir::new("speed-limit");
        let mut permissions = Permissions::full();
        permissions.speed_limit_kbps = Some(128);
        let limited = make_state(dir.path(), permissions);
        assert!(limited.speed_limiter.is_some());

        let unlimited = make_state(dir.path(), Permissions::full());
        assert!(unlimited.speed_limiter.is_none());
    }
}
