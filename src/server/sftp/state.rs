use anyhow::Result;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use super::packet::*;
use super::packet::SSH_FX_OK;
use super::packet::SSH_FX_EOF;
use super::packet::SSH_FX_NO_SUCH_FILE;
use super::packet::SSH_FX_PERMISSION_DENIED;
use super::packet::SSH_FX_FAILURE;
use super::packet::SSH_FX_OP_UNSUPPORTED;
use crate::core::logger::Logger;
use crate::core::users::UserManager;
use crate::core::users::Permissions;
use crate::core::file_logger::FileLogger;
use crate::server::common::utils::safe_resolve_path;

const SSH_FXP_INIT: u8 = 1;
#[allow(dead_code)]
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
    pub file: std::fs::File,
    pub locked: bool,
    pub existed: bool,
    pub written_bytes: u64,
    pub is_dir: bool,
    pub dir_entries: Vec<(String, bool, u64)>,
    pub dir_index: usize,
}

pub struct SftpState {
    pub home_dir: String,
    pub username: Option<String>,
    pub user_manager: Arc<StdMutex<UserManager>>,
    pub logger: Arc<StdMutex<Logger>>,
    pub file_logger: Arc<StdMutex<FileLogger>>,
    pub handles: HashMap<String, SftpFileHandle>,
    pub next_handle_id: u32,
    pub sftp_version: u32,
    pub buffer: Vec<u8>,
    pub locked_files: HashSet<PathBuf>,
    pub client_ip: String,
    cached_permissions: Option<Permissions>,
}

impl Drop for SftpState {
    fn drop(&mut self) {
        let locked_handles: Vec<(PathBuf, std::fs::File)> = self.handles.drain()
            .filter_map(|(_, handle)| {
                if handle.locked {
                    Some((handle.path, handle.file))
                } else {
                    None
                }
            })
            .collect();
        
        self.locked_files.clear();
        
        if locked_handles.is_empty() {
            return;
        }
        
        let logger = Arc::clone(&self.logger);
        
        let _ = std::thread::Builder::new()
            .name("sftp-cleanup".to_string())
            .spawn(move || {
                for (path, file) in locked_handles {
                    match fs2::FileExt::unlock(&file) {
                        Ok(()) => {
                            if let Ok(mut log) = logger.lock() {
                                log.info("SFTP", &format!("Auto-unlocked file on drop: {:?}", path));
                            }
                        }
                        Err(e) => {
                            if let Ok(mut log) = logger.lock() {
                                log.warning("SFTP", &format!("Failed to unlock file {:?}: {}", path, e));
                            }
                        }
                    }
                    drop(file);
                }
            });
    }
}

impl SftpState {
    pub fn new(
        home_dir: String,
        username: Option<String>,
        user_manager: Arc<StdMutex<UserManager>>,
        logger: Arc<StdMutex<Logger>>,
        file_logger: Arc<StdMutex<FileLogger>>,
        client_ip: String,
    ) -> Self {
        let cached_permissions = username.as_ref().and_then(|u| {
            user_manager.lock().ok().and_then(|mgr| {
                mgr.get_user(u).map(|user| user.permissions)
            })
        });
        
        SftpState {
            home_dir,
            username,
            user_manager,
            logger,
            file_logger,
            handles: HashMap::new(),
            next_handle_id: 0,
            sftp_version: 3,
            buffer: Vec::new(),
            locked_files: HashSet::new(),
            client_ip,
            cached_permissions,
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
                if let Ok(mut log) = self.logger.lock() {
                    log.warning("SFTP", "Permission check failed: username is None");
                }
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
                if !result
                    && let Ok(mut log) = self.logger.lock() {
                        log.warning("SFTP", &format!("Permission denied for user: {}", username));
                    }
                result
            }
            None => {
                if let Ok(mut log) = self.logger.lock() {
                    log.warning("SFTP", &format!("User not found in permission check: {}", username));
                }
                false
            }
        }
    }

    async fn handle_sftp_packet(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        if data.is_empty() {
            return Ok(build_status_packet(0, SSH_FX_FAILURE, "Bad packet: empty data", ""));
        }

        if data.len() < 5 {
            return Ok(build_status_packet(0, SSH_FX_FAILURE, "Bad packet: too short", ""));
        }

        let msg_type = data[0];
        
        if let Ok(mut log) = self.logger.lock() {
            log.debug("SFTP", &format!("[PACKET] type={}, len={}, id={}", msg_type, data.len(), parse_u32(data, 1)));
        }

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

        self.sftp_version = version.min(6);

        let mut payload = vec![2];
        payload.extend_from_slice(&self.sftp_version.to_be_bytes());
        
        let extensions = [
            ("posix-rename@openssh.com", "1"),
            ("statvfs@openssh.com", "2"),
            ("fstatvfs@openssh.com", "2"),
            ("hardlink@openssh.com", "1"),
            ("fsync@openssh.com", "1"),
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

        if !full_path.exists() {
            if let Ok(mut log) = self.logger.lock() {
                log.warning("SFTP", &format!("OPENDIR: Directory not found: {}", full_path.display()));
            }
            return Ok(build_status_packet(id, SSH_FX_NO_SUCH_FILE, "No such directory", ""));
        }

        if !full_path.is_dir() {
            if let Ok(mut log) = self.logger.lock() {
                log.warning("SFTP", &format!("OPENDIR: Path is not a directory: {}", full_path.display()));
            }
            return Ok(build_status_packet(id, SSH_FX_FAILURE, "Not a directory", ""));
        }

        let handle = self.generate_handle();
        self.handles.insert(handle.clone(), SftpFileHandle {
            path: full_path,
            file: std::fs::File::open("/dev/null")?,
            locked: false,
            existed: true,
            written_bytes: 0,
            is_dir: true,
            dir_entries: Vec::new(),
            dir_index: 0,
        });

        Ok(build_handle_packet(id, &handle))
    }

    async fn handle_close(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32_checked(data, 1)?;
        let (handle_str, _) = parse_string_checked(data, 5)?;

        if let Some(handle) = self.handles.remove(&handle_str)
            && !handle.is_dir && handle.written_bytes > 0 {
                let file_size = std::fs::metadata(&handle.path).map(|m| m.len()).unwrap_or(handle.written_bytes);
                
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

                if let Ok(mut log) = self.logger.lock() {
                    log.client_action(
                        "SFTP",
                        &format!("Closed file: {} ({} bytes)", handle.path.display(), file_size),
                        &self.client_ip,
                        self.username.as_deref(),
                        "CLOSE",
                    );
                }
            }

        Ok(build_status_packet(id, SSH_FX_OK, "OK", ""))
    }

    async fn handle_readdir(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32_checked(data, 1)?;
        let (handle_str, _) = parse_string_checked(data, 5)?;

        let handle = self.handles.get_mut(&handle_str);
        match handle {
            Some(h) if h.is_dir => {
                if h.dir_entries.is_empty() {
                    let mut read_entries = Vec::new();
                    if let Ok(mut dir) = std::fs::read_dir(&h.path) {
                        for entry in dir.by_ref() {
                            let entry = match entry {
                                Ok(e) => e,
                                Err(_) => continue,
                            };
                            let name = entry.file_name().to_string_lossy().to_string();
                            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
                            let size = entry.metadata().map(|m| m.len()).unwrap_or(1);
                            read_entries.push((name, is_dir, size));
                        }
                    }
                    h.dir_entries = read_entries;
                    h.dir_index = 0;
                }

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
                    
                    let long_name = format!("{} 1 user user {:>10} Jan 01 00:00 {}", 
                        if is_dir { "drwxr-xr-x" } else { "-rw-r--r--" },
                        size, name
                    );
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
        
        let offset_pos = 5 + handle_len;
        if data.len() < offset_pos + 12 {
            return Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid packet: too short for read", ""));
        }
        
        let offset = parse_u64_checked(data, offset_pos)?;
        let len = parse_u32_checked(data, offset_pos + 8)? as usize;

        if !self.check_permission_cached(|p| p.can_read) {
            return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, "Permission denied", ""));
        }

        let handle = self.handles.get_mut(&handle_str);
        match handle {
            Some(h) if !h.is_dir => {
                use std::io::{Read, Seek};
                
                if let Err(e) = h.file.seek(std::io::SeekFrom::Start(offset)) {
                    if let Ok(mut log) = self.logger.lock() {
                        log.warning("SFTP", &format!("Failed to seek to offset {}: {}", offset, e));
                    }
                    return Ok(build_status_packet(id, SSH_FX_FAILURE, "Seek failed", ""));
                }
                
                let read_len = len.min(32768);
                let mut buffer = vec![0u8; read_len];
                let n = match h.file.read(&mut buffer) {
                    Ok(n) => n,
                    Err(e) => {
                        if let Ok(mut log) = self.logger.lock() {
                            log.warning("SFTP", &format!("Failed to read from {:?}: {}", h.path, e));
                        }
                        return Ok(build_status_packet(id, SSH_FX_FAILURE, "Read failed", ""));
                    }
                };

                if n == 0 {
                    return Ok(build_status_packet(id, SSH_FX_EOF, "End of file", ""));
                }

                buffer.truncate(n);

                if let Ok(mut log) = self.logger.lock() {
                    log.client_action(
                        "SFTP",
                        &format!("Read {} bytes from {:?}", n, h.path),
                        &self.client_ip,
                        self.username.as_deref(),
                        "READ",
                    );
                }

                if let Ok(mut fl) = self.file_logger.lock() {
                    fl.log_download(
                        self.username.as_deref().unwrap_or("anonymous"),
                        &self.client_ip,
                        &h.path.to_string_lossy(),
                        n as u64,
                        "SFTP",
                    );
                }

                Ok(build_data_packet(id, &buffer))
            }
            _ => Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid handle", "")),
        }
    }

    async fn handle_write(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32_checked(data, 1)?;
        let (handle_str, handle_len) = parse_string_checked(data, 5)?;
        
        let offset_pos = 5 + handle_len;
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
        } else {
            if !self.check_permission_cached(|p| p.can_write) {
                return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, "Permission denied", ""));
            }
        }

        let handle = self.handles.get_mut(&handle_str);
        match handle {
            Some(h) if !h.is_dir => {
                use std::io::{Seek, Write};
                let _ = h.file.seek(std::io::SeekFrom::Start(offset));
                h.file.write_all(write_data)?;

                h.written_bytes += data_len as u64;

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

        match std::fs::remove_file(&full_path) {
            Ok(_) => {
                if let Ok(mut fl) = self.file_logger.lock() {
                    fl.log_delete(
                        self.username.as_deref().unwrap_or("anonymous"),
                        &self.client_ip,
                        &full_path.to_string_lossy(),
                        "SFTP",
                    );
                }
                if let Ok(mut log) = self.logger.lock() {
                    log.client_action(
                        "SFTP",
                        &format!("Removed file: {}", path),
                        &self.client_ip,
                        self.username.as_deref(),
                        "DELETE",
                    );
                }
                Ok(build_status_packet(id, SSH_FX_OK, "OK", ""))
            }
            Err(e) => {
                Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Failed to remove file: {}", e), ""))
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

        match std::fs::create_dir_all(&full_path) {
            Ok(_) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(&full_path, std::fs::Permissions::from_mode(0o755));
                }

                if let Ok(mut fl) = self.file_logger.lock() {
                    fl.log_mkdir(
                        self.username.as_deref().unwrap_or("anonymous"),
                        &self.client_ip,
                        &full_path.to_string_lossy(),
                        "SFTP",
                    );
                }
                if let Ok(mut log) = self.logger.lock() {
                    log.client_action(
                        "SFTP",
                        &format!("Created directory: {}", path),
                        &self.client_ip,
                        self.username.as_deref(),
                        "MKDIR",
                    );
                }
                Ok(build_status_packet(id, SSH_FX_OK, "OK", ""))
            }
            Err(e) => {
                let error_msg = format!("Failed to create directory: {} (path: {})", e, full_path.display());
                Ok(build_status_packet(id, SSH_FX_FAILURE, &error_msg, ""))
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

        if std::fs::remove_dir_all(&full_path).is_ok() {
            if let Ok(mut fl) = self.file_logger.lock() {
                fl.log_rmdir(
                    self.username.as_deref().unwrap_or("anonymous"),
                    &self.client_ip,
                    &full_path.to_string_lossy(),
                    "SFTP",
                );
            }
            if let Ok(mut log) = self.logger.lock() {
                log.client_action(
                    "SFTP",
                    &format!("Removed directory: {}", path),
                    &self.client_ip,
                    self.username.as_deref(),
                    "RMDIR",
                );
            }
            Ok(build_status_packet(id, SSH_FX_OK, "OK", ""))
        } else {
            Ok(build_status_packet(id, SSH_FX_FAILURE, "Failed to remove directory", ""))
        }
    }

    async fn handle_rename(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32_checked(data, 1)?;
        
        if let Ok(mut log) = self.logger.lock() {
            log.debug("SFTP", &format!("[RENAME] Starting, packet len={}, id={}", data.len(), id));
        }
        
        if data.len() < 9 {
            if let Ok(mut log) = self.logger.lock() {
                log.warning("SFTP", &format!("[RENAME] Invalid packet: len={}", data.len()));
            }
            return Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid packet", ""));
        }
        
        let (old_path, old_len) = parse_string_checked(data, 5)?;
        let new_path_pos = 5 + old_len;
        
        if let Ok(mut log) = self.logger.lock() {
            log.debug("SFTP", &format!("[RENAME] old_path='{}', old_len={}, new_path_pos={}", old_path, old_len, new_path_pos));
        }
        
        if data.len() < new_path_pos + 4 {
            if let Ok(mut log) = self.logger.lock() {
                log.warning("SFTP", &format!("[RENAME] Invalid packet: len={}, new_path_pos={}", data.len(), new_path_pos));
            }
            return Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid packet", ""));
        }
        
        let (new_path, new_len) = parse_string_checked(data, new_path_pos)?;

        if let Ok(mut log) = self.logger.lock() {
            log.debug("SFTP", &format!("[RENAME] new_path='{}', new_len={}", new_path, new_len));
        }

        if !self.check_permission_cached(|p| p.can_rename) {
            if let Ok(mut log) = self.logger.lock() {
                log.warning("SFTP", "[RENAME] Permission denied");
            }
            return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, "Permission denied", ""));
        }

        let old_full = match self.resolve_path(&old_path) {
            Ok(p) => p,
            Err(e) => {
                if let Ok(mut log) = self.logger.lock() {
                    log.warning("SFTP", &format!("[RENAME] Old path resolution failed: {}", e));
                }
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Path resolution failed: {}", e), ""));
            }
        };
        
        self.log_path_info("RENAME_OLD", &old_full);
        
        if !old_full.exists() {
            if let Ok(mut log) = self.logger.lock() {
                log.warning("SFTP", &format!("[RENAME] Old file not found: {}", old_full.display()));
            }
            return Ok(build_status_packet(id, SSH_FX_NO_SUCH_FILE, "No such file", ""));
        }
        
        let new_full = match self.resolve_path(&new_path) {
            Ok(p) => p,
            Err(e) => {
                if let Ok(mut log) = self.logger.lock() {
                    log.warning("SFTP", &format!("[RENAME] New path resolution failed: {}", e));
                }
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Path resolution failed: {}", e), ""));
            }
        };
        
        self.log_path_info("RENAME_NEW", &new_full);

        if let Ok(mut log) = self.logger.lock() {
            log.debug("SFTP", &format!("[RENAME] Attempting rename: '{}' -> '{}'", old_full.display(), new_full.display()));
        }

        let result = match std::fs::rename(&old_full, &new_full) {
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
                if let Ok(mut log) = self.logger.lock() {
                    log.client_action(
                        "SFTP",
                        &format!("Renamed: {} -> {}", old_path, new_path),
                        &self.client_ip,
                        self.username.as_deref(),
                        "RENAME",
                    );
                }
                if let Ok(mut log) = self.logger.lock() {
                    log.debug("SFTP", "[RENAME] Success, returning SSH_FX_OK");
                }
                build_status_packet(id, SSH_FX_OK, "OK", "")
            }
            Err(e) => {
                if let Ok(mut log) = self.logger.lock() {
                    log.warning("SFTP", &format!("[RENAME] Failed: {}", e));
                }
                build_status_packet(id, SSH_FX_FAILURE, &format!("Failed to rename: {}", e), "")
            }
        };
        
        if let Ok(mut log) = self.logger.lock() {
            log.debug("SFTP", &format!("[RENAME] Returning response, len={}", result.len()));
        }
        
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

        match std::fs::metadata(&full_path) {
            Ok(metadata) => {
                let mut payload = vec![105];
                payload.extend_from_slice(&id.to_be_bytes());
                payload.extend_from_slice(&build_attrs(metadata.is_dir(), metadata.len()));
                Ok(build_packet(&payload))
            }
            Err(e) => {
                if let Ok(mut log) = self.logger.lock() {
                    log.warning("SFTP", &format!("STAT: Failed to get metadata for {}: {}", full_path.display(), e));
                }
                Ok(build_status_packet(id, SSH_FX_NO_SUCH_FILE, "No such file", ""))
            }
        }
    }

    async fn handle_lstat(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        self.handle_stat(data).await
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

        let attrs_offset = 5 + path_len;
        if data.len() < attrs_offset + 4 {
            return Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid SETSTAT packet", ""));
        }

        let flags = parse_u32_checked(data, attrs_offset)?;

        if flags & 0x00000004 != 0 && data.len() >= attrs_offset + 4 + 8 + 8 + 4 {
            let permissions = parse_u32_checked(data, attrs_offset + 4 + 8 + 8)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&full_path, std::fs::Permissions::from_mode(permissions));
            }
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
                let attrs_offset = 5 + handle_len;
                if data.len() < attrs_offset + 4 {
                    return Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid FSETSTAT packet", ""));
                }

                let flags = parse_u32_checked(data, attrs_offset)?;

                if flags & 0x00000004 != 0 && data.len() >= attrs_offset + 4 + 8 + 8 + 4 {
                    let permissions = parse_u32_checked(data, attrs_offset + 4 + 8 + 8)?;
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let _ = std::fs::set_permissions(&h.path, std::fs::Permissions::from_mode(permissions));
                    }
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
                match std::fs::metadata(&h.path) {
                    Ok(metadata) => {
                        let mut payload = vec![105];
                        payload.extend_from_slice(&id.to_be_bytes());
                        payload.extend_from_slice(&build_attrs(metadata.is_dir(), metadata.len()));
                        Ok(build_packet(&payload))
                    }
                    Err(_) => Ok(build_status_packet(id, SSH_FX_NO_SUCH_FILE, "No such file", "")),
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

        let resolved = if full_path.exists() {
            full_path.canonicalize().unwrap_or(full_path)
        } else {
            full_path
        };

        let home = PathBuf::from(&self.home_dir);
        let home_canon = home.canonicalize().unwrap_or(home);
        
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

        let mut payload = vec![104];
        payload.extend_from_slice(&id.to_be_bytes());
        payload.extend_from_slice(&1u32.to_be_bytes());
        payload.extend_from_slice(&(path_str.len() as u32).to_be_bytes());
        payload.extend_from_slice(path_str.as_bytes());
        let longname = format!("drwxr-xr-x  1 user user  0 Jan 01 00:00 {}", path_str);
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
        let exists = path.exists();
        let is_dir = path.is_dir();
        let is_file = path.is_file();
        
        let canonical = if exists {
            path.canonicalize().ok()
        } else {
            None
        };
        
        if let Ok(mut log) = self.logger.lock() {
            log.debug(
                "SFTP",
                &format!(
                    "[{}] Path info: path='{}', exists={}, is_dir={}, is_file={}, canonical='{}'",
                    operation,
                    path.display(),
                    exists,
                    is_dir,
                    is_file,
                    canonical.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "N/A".to_string())
                ),
            );
        }
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
                if let Ok(mut log) = self.logger.lock() {
                    log.warning("SFTP", "Failed to generate unique handle after 1000 attempts");
                }
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
        let pflags_pos = 5 + path_len;
        
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
        
        let file_existed = full_path.exists();

        if need_excl && need_creat && file_existed {
            return Ok(build_status_packet(id, SSH_FX_FAILURE, "File already exists", ""));
        }
        
        if need_read && !need_write && !file_existed {
            return Ok(build_status_packet(id, SSH_FX_NO_SUCH_FILE, &format!("File not found: {}", full_path.display()), ""));
        }

        let file_result = if need_write {
            let mut opts = std::fs::OpenOptions::new();
            opts.read(need_read).write(true);
            
            if need_trunc && need_creat {
                opts.create(true).truncate(true);
            } else if need_append {
                opts.create(need_creat).append(true);
            } else if need_creat {
                opts.create(true).truncate(false);
            }
            
            opts.open(&full_path)
        } else {
            std::fs::File::open(&full_path)
        };

        match file_result {
            Ok(file) => {
                if !file_existed {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let _ = std::fs::set_permissions(&full_path, std::fs::Permissions::from_mode(0o644));
                    }
                }

                let handle = self.generate_handle();
                self.handles.insert(handle.clone(), SftpFileHandle {
                    path: full_path,
                    file,
                    locked: false,
                    existed: file_existed,
                    written_bytes: 0,
                    is_dir: false,
                    dir_entries: Vec::new(),
                    dir_index: 0,
                });
                Ok(build_handle_packet(id, &handle))
            }
            Err(e) => {
                let error_msg = format!("Failed to open file: {} (path: {})", e, full_path.display());
                Ok(build_status_packet(id, SSH_FX_FAILURE, &error_msg, ""))
            }
        }
    }

    async fn handle_readlink(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32_checked(data, 1)?;
        let (path, path_len) = parse_string_checked(data, 5)?;
        
        if let Ok(mut log) = self.logger.lock() {
            log.debug("SFTP", &format!("[READLINK] id={}, path='{}', path_len={}", id, path, path_len));
        }

        let full_path = match self.resolve_path(&path) {
            Ok(p) => p,
            Err(e) => {
                if let Ok(mut log) = self.logger.lock() {
                    log.warning("SFTP", &format!("[READLINK] Path resolution failed: {}", e));
                }
                return Ok(build_status_packet(id, SSH_FX_NO_SUCH_FILE, "No such file", ""));
            }
        };
        
        if let Ok(mut log) = self.logger.lock() {
            log.debug("SFTP", &format!("[READLINK] resolved_path='{}', exists={}", full_path.display(), full_path.exists()));
        }

        if !full_path.exists() {
            return Ok(build_status_packet(id, SSH_FX_NO_SUCH_FILE, "No such file", ""));
        }

        match std::fs::symlink_metadata(&full_path) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    match std::fs::read_link(&full_path) {
                        Ok(target) => {
                            let target_str = target.to_string_lossy().to_string();
                            if let Ok(mut log) = self.logger.lock() {
                                log.debug("SFTP", &format!("[READLINK] symlink target='{}'", target_str));
                            }
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
                            if let Ok(mut log) = self.logger.lock() {
                                log.warning("SFTP", &format!("[READLINK] Failed to read link: {}", e));
                            }
                            Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Failed to read link: {}", e), ""))
                        }
                    }
                } else {
                    if let Ok(mut log) = self.logger.lock() {
                        log.debug("SFTP", "[READLINK] Not a symlink, returning SSH_FX_FAILURE");
                    }
                    Ok(build_status_packet(id, SSH_FX_FAILURE, "Not a symbolic link", ""))
                }
            }
            Err(e) => {
                if let Ok(mut log) = self.logger.lock() {
                    log.warning("SFTP", &format!("READLINK: symlink_metadata failed for {}: {}", full_path.display(), e));
                }
                Ok(build_status_packet(id, SSH_FX_NO_SUCH_FILE, "No such file", ""))
            }
        }
    }

    async fn handle_symlink(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32_checked(data, 1)?;
        let (target, target_len) = parse_string_checked(data, 5)?;
        let link_pos = 5 + target_len;
        let (link_path, _) = parse_string_checked(data, link_pos)?;

        if !self.check_permission_cached(|p| p.can_write) {
            return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, "Permission denied", ""));
        }

        let full_link = match self.resolve_path(&link_path) {
            Ok(p) => p,
            Err(e) => {
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Path resolution failed: {}", e), ""));
            }
        };
        let home = PathBuf::from(&self.home_dir);
        let home_canon = home.canonicalize().unwrap_or(home);
        
        let full_target = if target.starts_with('/') {
            let resolved = match safe_resolve_path(&self.home_dir, &target) {
                Ok(p) => p,
                Err(e) => {
                    return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Path resolution failed: {}", e), ""));
                }
            };
            if !resolved.starts_with(&home_canon) {
                if let Ok(mut log) = self.logger.lock() {
                    log.client_action(
                        "SFTP",
                        "Symlink rejected: absolute target outside home directory",
                        &self.client_ip,
                        self.username.as_deref(),
                        "SYMLINK_DENIED",
                    );
                }
                return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, "Permission denied: target outside home directory", ""));
            }
            resolved
        } else {
            let resolved = match self.resolve_path(&target) {
                Ok(p) => p,
                Err(e) => {
                    return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Path resolution failed: {}", e), ""));
                }
            };
            if !resolved.starts_with(&home_canon) {
                if let Ok(mut log) = self.logger.lock() {
                    log.client_action(
                        "SFTP",
                        "Symlink rejected: relative target outside home directory",
                        &self.client_ip,
                        self.username.as_deref(),
                        "SYMLINK_DENIED",
                    );
                }
                return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, "Permission denied: target outside home directory", ""));
            }
            resolved
        };

        if std::os::unix::fs::symlink(&full_target, &full_link).is_ok() {
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
            if let Ok(mut log) = self.logger.lock() {
                log.client_action(
                    "SFTP",
                    &format!("Created symlink: {} -> {}", link_path, target),
                    &self.client_ip,
                    self.username.as_deref(),
                    "SYMLINK",
                );
            }
            Ok(build_status_packet(id, SSH_FX_OK, "OK", ""))
        } else {
            Ok(build_status_packet(id, SSH_FX_FAILURE, "Failed to create symlink", ""))
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

                match fs2::FileExt::lock_exclusive(&h.file) {
                    Ok(()) => {
                        h.locked = true;
                        self.locked_files.insert(h.path.clone());
                        if let Ok(mut log) = self.logger.lock() {
                            log.client_action(
                                "SFTP",
                                &format!("Locked file: {:?}", h.path),
                                &self.client_ip,
                                self.username.as_deref(),
                                "LOCK",
                            );
                        }
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

                match fs2::FileExt::unlock(&h.file) {
                    Ok(()) => {
                        h.locked = false;
                        self.locked_files.remove(&h.path);
                        if let Ok(mut log) = self.logger.lock() {
                            log.client_action(
                                "SFTP",
                                &format!("Unlocked file: {:?}", h.path),
                                &self.client_ip,
                                self.username.as_deref(),
                                "UNLOCK",
                            );
                        }
                        Ok(build_status_packet(id, SSH_FX_OK, "OK", ""))
                    }
                    Err(_) => Ok(build_status_packet(id, SSH_FX_FAILURE, "Failed to unlock file", "")),
                }
            }
            _ => Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid handle", "")),
        }
    }
}
