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

pub enum SftpFileHandle {
    File {
        path: PathBuf,
        file: tokio::fs::File,
        locked: bool,
        existed: bool,
    },
    Dir {
        path: PathBuf,
        entries: Vec<(String, bool, u64)>,
        index: usize,
    },
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
}

impl Drop for SftpState {
    fn drop(&mut self) {
        let locked_handles: Vec<(PathBuf, tokio::fs::File)> = self.handles.drain()
            .filter_map(|(_, handle)| {
                if let SftpFileHandle::File { path, file, locked, .. } = handle {
                    if locked {
                        Some((path, file))
                    } else {
                        None
                    }
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
        
        match std::thread::Builder::new()
            .name("sftp-cleanup".to_string())
            .spawn(move || {
                Self::cleanup_locked_files(locked_handles, logger);
            }) 
        {
            Ok(handle) => {
                if let Err(e) = handle.join() {
                    if let Ok(mut log) = self.logger.lock() {
                        log.warning("SFTP", &format!("Cleanup thread panicked: {:?}", e));
                    }
                }
            }
            Err(e) => {
                if let Ok(mut log) = self.logger.lock() {
                    log.warning("SFTP", &format!("Failed to spawn cleanup thread: {}", e));
                }
            }
        }
    }
}

impl SftpState {
    fn cleanup_locked_files(locked_handles: Vec<(PathBuf, tokio::fs::File)>, logger: Arc<StdMutex<Logger>>) {
        for (path, file) in locked_handles {
            let unlocked = match tokio::runtime::Handle::try_current() {
                Ok(handle) => {
                    handle.block_on(async {
                        let std_file = file.into_std().await;
                        fs2::FileExt::unlock(&std_file).is_ok()
                    })
                }
                Err(_) => {
                    match tokio::runtime::Runtime::new() {
                        Ok(rt) => rt.block_on(async {
                            let std_file = file.into_std().await;
                            fs2::FileExt::unlock(&std_file).is_ok()
                        }),
                        Err(_) => false,
                    }
                }
            };
            
            if unlocked {
                if let Ok(mut log) = logger.lock() {
                    log.info("SFTP", &format!("Auto-unlocked file on drop: {:?}", path));
                }
            }
        }
    }

    pub async fn process_sftp_data(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        self.buffer.extend_from_slice(data);
        
        const MAX_PACKET_SIZE: usize = 256 * 1024;
        
        while self.buffer.len() >= 4 {
            let packet_len = u32::from_be_bytes([
                self.buffer[0], self.buffer[1], self.buffer[2], self.buffer[3]
            ]) as usize;
            
            if packet_len == 0 {
                self.buffer.clear();
                return Ok(build_status_packet(0, SSH_FX_FAILURE, "Invalid packet length", ""));
            }
            
            if packet_len > MAX_PACKET_SIZE {
                if self.buffer.len() >= 4 + packet_len {
                    let packet_data = &self.buffer[4..4 + packet_len];
                    let response = if !packet_data.is_empty() {
                        let id = if packet_data.len() >= 5 {
                            parse_u32(packet_data, 1)
                        } else {
                            0
                        };
                        self.logger.lock().unwrap().warning(
                            "SFTP",
                            &format!("Dropping oversized packet: {} bytes (id: {})", packet_len, id),
                        );
                        build_status_packet(id, SSH_FX_FAILURE, "Packet too large", "")
                    } else {
                        Vec::new()
                    };
                    self.buffer.drain(..4 + packet_len);
                    if !response.is_empty() {
                        return Ok(response);
                    }
                    continue;
                } else {
                    self.buffer.clear();
                    return Ok(build_status_packet(0, SSH_FX_FAILURE, "Packet too large", ""));
                }
            }
            
            if self.buffer.len() < 4 + packet_len {
                break;
            }
            
            let packet: Vec<u8> = self.buffer[4..4 + packet_len].to_vec();
            self.buffer.drain(0..4 + packet_len);
            
            if !packet.is_empty() {
                let response = self.handle_sftp_packet(&packet).await?;
                return Ok(response);
            }
        }
        
        Ok(Vec::new())
    }

    pub fn check_permission(&self, check_fn: impl Fn(&crate::core::users::Permissions) -> bool) -> bool {
        let (username, logger) = {
             let username = self.username.clone();
             (username, Arc::clone(&self.logger))
         };
        
        if let Some(username) = &username {
            let users = self.user_manager.lock().unwrap();
            if let Some(user) = users.get_user(username) {
                let result = check_fn(&user.permissions);
                if !result {
                    logger.lock().unwrap().warning(
                        "SFTP",
                        &format!("Permission denied for user: {}", username),
                    );
                }
                return result;
            } else {
                logger.lock().unwrap().warning(
                    "SFTP",
                    &format!("User not found in permission check: {}", username),
                );
            }
        } else {
            logger.lock().unwrap().warning(
                "SFTP",
                "Permission check failed: username is None",
            );
        }
        false
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

        self.sftp_version = version.min(6);

        let mut payload = vec![2];
        payload.extend_from_slice(&self.sftp_version.to_be_bytes());
        Ok(build_packet(&payload))
    }

    async fn handle_opendir(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32(data, 1);
        let path = parse_string(data, 5)?;

        if !self.check_permission(|p| p.can_list) {
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
            self.logger.lock().unwrap().warning(
                "SFTP",
                &format!("OPENDIR: Directory not found: {}", full_path.display()),
            );
            return Ok(build_status_packet(id, SSH_FX_NO_SUCH_FILE, "No such directory", ""));
        }

        if !full_path.is_dir() {
            self.logger.lock().unwrap().warning(
                "SFTP",
                &format!("OPENDIR: Path is not a directory: {}", full_path.display()),
            );
            return Ok(build_status_packet(id, SSH_FX_FAILURE, "Not a directory", ""));
        }

        let handle = self.generate_handle();
        self.handles.insert(handle.clone(), SftpFileHandle::Dir {
            path: full_path,
            entries: Vec::new(),
            index: 0,
        });

        Ok(build_handle_packet(id, &handle))
    }

    async fn handle_close(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32(data, 1);
        let handle = parse_string(data, 5)?;

        self.handles.remove(&handle);
        Ok(build_status_packet(id, SSH_FX_OK, "OK", ""))
    }

    async fn handle_readdir(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32(data, 1);
        let handle_str = parse_string(data, 5)?;

        let entries_result = {
            let handle = self.handles.get_mut(&handle_str);
            match handle {
                Some(SftpFileHandle::Dir { path, entries, index }) => {
                    if entries.is_empty() {
                        let mut read_entries = Vec::new();
                        if let Ok(mut dir) = tokio::fs::read_dir(path).await {
                            while let Ok(Some(entry)) = dir.next_entry().await {
                                let name = entry.file_name().to_string_lossy().to_string();
                                let is_dir = entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false);
                                let size = entry.metadata().await.map(|m| m.len()).unwrap_or(0);
                                read_entries.push((name, is_dir, size));
                            }
                        }
                        *entries = read_entries;
                        *index = 0;
                    }

                    if *index >= entries.len() {
                        return Ok(build_status_packet(id, SSH_FX_EOF, "End of directory", ""));
                    }

                    let count = (entries.len() - *index).min(100);
                    let result_entries: Vec<(String, bool, u64)> = entries[*index..*index + count].to_vec();
                    *index += count;
                    Some(result_entries)
                }
                _ => None,
            }
        };

        match entries_result {
            Some(dir_entries) => {
                let mut payload = vec![104];
                payload.extend_from_slice(&id.to_be_bytes());
                payload.extend_from_slice(&(dir_entries.len() as u32).to_be_bytes());

                for (name, is_dir, size) in dir_entries {
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
            None => Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid handle", "")),
        }
    }

    async fn handle_read(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32(data, 1);
        let handle_str = parse_string(data, 5)?;
        let offset = parse_u64(data, 5 + 4 + handle_str.len());
        let len = parse_u32(data, 5 + 4 + handle_str.len() + 8) as usize;

        if !self.check_permission(|p| p.can_read) {
            return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, "Permission denied", ""));
        }

        let handle = self.handles.get_mut(&handle_str);
        match handle {
            Some(SftpFileHandle::File { path, file, .. }) => {
                use tokio::io::{AsyncSeekExt, AsyncReadExt};
                let _ = file.seek(std::io::SeekFrom::Start(offset)).await;
                
                let mut buffer = vec![0u8; len.min(32768)];
                let n = file.read(&mut buffer).await.unwrap_or(0);
                buffer.truncate(n);

                self.logger.lock().unwrap().client_action(
                    "SFTP",
                    &format!("Read {} bytes from {:?}", n, path),
                    &self.client_ip,
                    self.username.as_deref(),
                    "READ",
                );

                if n > 0 {
                    self.file_logger.lock().unwrap().log_download(
                        self.username.as_deref().unwrap_or("anonymous"),
                        &self.client_ip,
                        &path.to_string_lossy(),
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
        let id = parse_u32(data, 1);
        let handle_str = parse_string(data, 5)?;
        let offset_pos = 5 + 4 + handle_str.len();
        let offset = parse_u64(data, offset_pos);
        let data_len = parse_u32(data, offset_pos + 8) as usize;
        let write_data = &data[offset_pos + 12..offset_pos + 12 + data_len];

        if offset > 0 {
            if !self.check_permission(|p| p.can_append) {
                return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, "Permission denied (append)", ""));
            }
        } else {
            if !self.check_permission(|p| p.can_write) {
                return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, "Permission denied", ""));
            }
        }

        let handle = self.handles.get_mut(&handle_str);
        match handle {
            Some(SftpFileHandle::File { path, file, existed, .. }) => {
                use tokio::io::{AsyncSeekExt, AsyncWriteExt};
                let _ = file.seek(std::io::SeekFrom::Start(offset)).await;
                file.write_all(write_data).await?;
                let _ = file.flush().await;

                self.logger.lock().unwrap().client_action(
                    "SFTP",
                    &format!("Wrote {} bytes to {:?}", data_len, path),
                    &self.client_ip,
                    self.username.as_deref(),
                    "WRITE",
                );

                if *existed {
                    self.file_logger.lock().unwrap().log_update(
                        self.username.as_deref().unwrap_or("anonymous"),
                        &self.client_ip,
                        &path.to_string_lossy(),
                        data_len as u64,
                        "SFTP",
                    );
                } else {
                    self.file_logger.lock().unwrap().log_upload(
                        self.username.as_deref().unwrap_or("anonymous"),
                        &self.client_ip,
                        &path.to_string_lossy(),
                        data_len as u64,
                        "SFTP",
                    );
                }

                Ok(build_status_packet(id, SSH_FX_OK, "OK", ""))
            }
            _ => Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid handle", "")),
        }
    }

    async fn handle_remove(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32(data, 1);
        let path = parse_string(data, 5)?;

        if !self.check_permission(|p| p.can_delete) {
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
                self.file_logger.lock().unwrap().log_delete(
                    self.username.as_deref().unwrap_or("anonymous"),
                    &self.client_ip,
                    &full_path.to_string_lossy(),
                    "SFTP",
                );
                self.logger.lock().unwrap().client_action(
                    "SFTP",
                    &format!("Removed file: {}", path),
                    &self.client_ip,
                    self.username.as_deref(),
                    "DELETE",
                );
                Ok(build_status_packet(id, SSH_FX_OK, "OK", ""))
            }
            Err(e) => {
                let error_msg = format!("Failed to remove file: {}", e);
                Ok(build_status_packet(id, SSH_FX_FAILURE, &error_msg, ""))
            }
        }
    }

    async fn handle_mkdir(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32(data, 1);
        let path = parse_string(data, 5)?;

        if !self.check_permission(|p| p.can_mkdir) {
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
                    if let Err(e) = tokio::fs::set_permissions(&full_path, std::fs::Permissions::from_mode(0o755)).await {
                        self.logger.lock().unwrap().warning(
                            "SFTP",
                            &format!("Failed to set directory permissions: {}", e),
                        );
                    }
                }

                self.file_logger.lock().unwrap().log_mkdir(
                    self.username.as_deref().unwrap_or("anonymous"),
                    &self.client_ip,
                    &full_path.to_string_lossy(),
                    "SFTP",
                );
                self.logger.lock().unwrap().client_action(
                    "SFTP",
                    &format!("Created directory: {}", path),
                    &self.client_ip,
                    self.username.as_deref(),
                    "MKDIR",
                );
                Ok(build_status_packet(id, SSH_FX_OK, "OK", ""))
            }
            Err(e) => {
                let error_msg = format!("Failed to create directory: {} (path: {})", e, full_path.display());
                self.logger.lock().unwrap().error(
                    "SFTP",
                    &error_msg,
                );
                Ok(build_status_packet(id, SSH_FX_FAILURE, &error_msg, ""))
            }
        }
    }

    async fn handle_rmdir(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32(data, 1);
        let path = parse_string(data, 5)?;

        if !self.check_permission(|p| p.can_rmdir) {
            return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, "Permission denied", ""));
        }

        let full_path = match self.resolve_path(&path) {
            Ok(p) => p,
            Err(e) => {
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Path resolution failed: {}", e), ""));
            }
        };

        if tokio::fs::remove_dir_all(&full_path).await.is_ok() {
            self.file_logger.lock().unwrap().log_rmdir(
                self.username.as_deref().unwrap_or("anonymous"),
                &self.client_ip,
                &full_path.to_string_lossy(),
                "SFTP",
            );
            self.logger.lock().unwrap().client_action(
                "SFTP",
                &format!("Removed directory: {}", path),
                &self.client_ip,
                self.username.as_deref(),
                "RMDIR",
            );
            Ok(build_status_packet(id, SSH_FX_OK, "OK", ""))
        } else {
            Ok(build_status_packet(id, SSH_FX_FAILURE, "Failed to remove directory", ""))
        }
    }

    async fn handle_rename(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32(data, 1);
        let old_path = parse_string(data, 5)?;
        let new_path_pos = 5 + 4 + old_path.len();
        let new_path = parse_string(data, new_path_pos)?;

        if !self.check_permission(|p| p.can_rename) {
            return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, "Permission denied", ""));
        }

        let old_full = match self.resolve_path(&old_path) {
            Ok(p) => p,
            Err(e) => {
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Path resolution failed: {}", e), ""));
            }
        };
        let new_full = match self.resolve_path(&new_path) {
            Ok(p) => p,
            Err(e) => {
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Path resolution failed: {}", e), ""));
            }
        };

        if tokio::fs::rename(&old_full, &new_full).await.is_ok() {
            self.file_logger.lock().unwrap().log_rename(
                self.username.as_deref().unwrap_or("anonymous"),
                &self.client_ip,
                &old_full.to_string_lossy(),
                &new_full.to_string_lossy(),
                "SFTP",
            );
            self.logger.lock().unwrap().client_action(
                "SFTP",
                &format!("Renamed: {} -> {}", old_path, new_path),
                &self.client_ip,
                self.username.as_deref(),
                "RENAME",
            );
            Ok(build_status_packet(id, SSH_FX_OK, "OK", ""))
        } else {
            Ok(build_status_packet(id, SSH_FX_FAILURE, "Failed to rename", ""))
        }
    }

    async fn handle_stat(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32(data, 1);
        let path = parse_string(data, 5)?;

        if !self.check_permission(|p| p.can_read) {
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
                self.logger.lock().unwrap().warning(
                    "SFTP",
                    &format!("STAT: Failed to get metadata for {}: {}", full_path.display(), e),
                );
                Ok(build_status_packet(id, SSH_FX_NO_SUCH_FILE, "No such file", ""))
            }
        }
    }

    async fn handle_lstat(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        self.handle_stat(data).await
    }

    async fn handle_setstat(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32(data, 1);
        let path = parse_string(data, 5)?;

        if !self.check_permission(|p| p.can_write) {
            return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, "Permission denied", ""));
        }

        let full_path = match self.resolve_path(&path) {
            Ok(p) => p,
            Err(e) => {
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Path resolution failed: {}", e), ""));
            }
        };

        let attrs_offset = 5 + 4 + path.len();
        if data.len() < attrs_offset + 4 {
            return Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid SETSTAT packet", ""));
        }

        let flags = parse_u32(data, attrs_offset);

        if flags & 0x00000004 != 0 {
            let permissions = parse_u32(data, attrs_offset + 4 + 8 + 8);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Err(e) = tokio::fs::set_permissions(&full_path, std::fs::Permissions::from_mode(permissions)).await {
                    self.logger.lock().unwrap().warning(
                        "SFTP",
                        &format!("Failed to set permissions: {}", e),
                    );
                }
            }
        }

        self.logger.lock().unwrap().client_action(
            "SFTP",
            &format!("Setstat: {}", path),
            &self.client_ip,
            self.username.as_deref(),
            "SETSTAT",
        );

        Ok(build_status_packet(id, SSH_FX_OK, "OK", ""))
    }

    async fn handle_fsetstat(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32(data, 1);
        let handle_str = parse_string(data, 5)?;

        if !self.check_permission(|p| p.can_write) {
            return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, "Permission denied", ""));
        }

        let handle = self.handles.get(&handle_str);
        match handle {
            Some(SftpFileHandle::File { path, .. }) => {
                let attrs_offset = 5 + 4 + handle_str.len();
                if data.len() < attrs_offset + 4 {
                    return Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid FSETSTAT packet", ""));
                }

                let flags = parse_u32(data, attrs_offset);

                if flags & 0x00000004 != 0 {
                    let permissions = parse_u32(data, attrs_offset + 4 + 8 + 8);
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        if let Err(e) = tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(permissions)).await {
                            self.logger.lock().unwrap().warning(
                                "SFTP",
                                &format!("Failed to set permissions: {}", e),
                            );
                        }
                    }
                }

                self.logger.lock().unwrap().client_action(
                    "SFTP",
                    &format!("Fsetstat: {:?}", path),
                    &self.client_ip,
                    self.username.as_deref(),
                    "FSETSTAT",
                );

                Ok(build_status_packet(id, SSH_FX_OK, "OK", ""))
            }
            _ => Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid handle", "")),
        }
    }

    async fn handle_fstat(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32(data, 1);
        let handle_str = parse_string(data, 5)?;

        if !self.check_permission(|p| p.can_read) {
            return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, "Permission denied", ""));
        }

        let handle = self.handles.get(&handle_str);
        match handle {
            Some(SftpFileHandle::File { path, .. }) => {
                match tokio::fs::metadata(path).await {
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
        let id = parse_u32(data, 1);
        let path = parse_string(data, 5)?;

        if !self.check_permission(|p| p.can_read) {
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
        
        let relative_path = if resolved.starts_with(&home_canon) {
            resolved.strip_prefix(&home_canon).unwrap_or(&resolved).to_path_buf()
        } else {
            resolved
        };
        
        let path_str = if relative_path.as_os_str().is_empty() {
            ".".to_string()
        } else {
            relative_path.to_string_lossy().to_string()
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
        
        self.logger.lock().unwrap().debug(
            "SFTP",
            &format!(
                "Path resolution: input='{}', home='{}', resolved='{}'",
                path, self.home_dir, resolved.display()
            ),
        );
        
        Ok(resolved)
    }
    
    #[allow(dead_code)]
    fn check_path_accessible(&self, path: &PathBuf) -> (bool, Option<String>) {
        match std::fs::metadata(path) {
            Ok(_) => (true, None),
            Err(e) => {
                let error_msg = format!("Path accessibility check failed: {} - {}", path.display(), e);
                self.logger.lock().unwrap().warning("SFTP", &error_msg);
                (false, Some(error_msg))
            }
        }
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
        
        self.logger.lock().unwrap().debug(
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
                self.logger.lock().unwrap().warning(
                    "SFTP",
                    "Failed to generate unique handle after 1000 attempts",
                );
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

        let id = parse_u32(data, 1);
        let path = parse_string(data, 5)?;
        let pflags_pos = 5 + 4 + path.len();
        let pflags = parse_u32(data, pflags_pos);

        let need_read = pflags & SSH_FXF_READ != 0;
        let need_write = pflags & SSH_FXF_WRITE != 0;
        let need_append = pflags & SSH_FXF_APPEND != 0;
        let need_creat = pflags & SSH_FXF_CREAT != 0;
        let need_trunc = pflags & SSH_FXF_TRUNC != 0;
        let need_excl = pflags & SSH_FXF_EXCL != 0;

        if !self.check_permission(|p| {
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
            self.logger.lock().unwrap().warning(
                "SFTP",
                &format!("OPEN: File already exists (EXCL): {}", full_path.display()),
            );
            return Ok(build_status_packet(id, SSH_FX_FAILURE, "File already exists", ""));
        }
        
        if need_read && !need_write && !file_existed {
            self.logger.lock().unwrap().warning(
                "SFTP",
                &format!("OPEN: File not found for reading: {}", full_path.display()),
            );
            return Ok(build_status_packet(id, SSH_FX_NO_SUCH_FILE, &format!("File not found: {}", full_path.display()), ""));
        }

        let file_result = if need_write {
            if need_trunc && need_creat {
                tokio::fs::OpenOptions::new()
                    .read(need_read)
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(&full_path).await
            } else if need_append {
                tokio::fs::OpenOptions::new()
                    .read(need_read)
                    .write(true)
                    .create(need_creat)
                    .append(true)
                    .open(&full_path).await
            } else if need_creat {
                tokio::fs::OpenOptions::new()
                    .read(need_read)
                    .write(true)
                    .create(true)
                    .truncate(false)
                    .open(&full_path).await
            } else {
                tokio::fs::OpenOptions::new()
                    .read(need_read)
                    .write(true)
                    .open(&full_path).await
            }
        } else {
            tokio::fs::File::open(&full_path).await
        };

        match file_result {
            Ok(file) => {
                if !file_existed {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        if let Err(e) = tokio::fs::set_permissions(&full_path, std::fs::Permissions::from_mode(0o644)).await {
                            self.logger.lock().unwrap().warning(
                                "SFTP",
                                &format!("Failed to set file permissions: {}", e),
                            );
                        }
                    }
                }

                let handle = self.generate_handle();
                self.handles.insert(handle.clone(), SftpFileHandle::File {
                    path: full_path,
                    file,
                    locked: false,
                    existed: file_existed,
                });
                Ok(build_handle_packet(id, &handle))
            }
            Err(e) => {
                let error_msg = format!("Failed to open file: {} (path: {})", e, full_path.display());
                self.logger.lock().unwrap().error(
                    "SFTP",
                    &error_msg,
                );
                Ok(build_status_packet(id, SSH_FX_FAILURE, &error_msg, ""))
            }
        }
    }

    async fn handle_readlink(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32(data, 1);
        let path = parse_string(data, 5)?;

        let full_path = match self.resolve_path(&path) {
            Ok(p) => p,
            Err(e) => {
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Path resolution failed: {}", e), ""));
            }
        };

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
            Err(_) => Ok(build_status_packet(id, SSH_FX_NO_SUCH_FILE, "No such file", "")),
        }
    }

    async fn handle_symlink(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32(data, 1);
        let target = parse_string(data, 5)?;
        let link_pos = 5 + 4 + target.len();
        let link_path = parse_string(data, link_pos)?;

        if !self.check_permission(|p| p.can_write) {
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
                self.logger.lock().unwrap().client_action(
                    "SFTP",
                    "Symlink rejected: absolute target outside home directory",
                    &self.client_ip,
                    self.username.as_deref(),
                    "SYMLINK_DENIED",
                );
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
                self.logger.lock().unwrap().client_action(
                    "SFTP",
                    "Symlink rejected: relative target outside home directory",
                    &self.client_ip,
                    self.username.as_deref(),
                    "SYMLINK_DENIED",
                );
                return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, "Permission denied: target outside home directory", ""));
            }
            resolved
        };

        if tokio::fs::symlink(&full_target, &full_link).await.is_ok() {
            self.file_logger.lock().unwrap().log(crate::core::file_logger::FileLogInfo {
                username: self.username.as_deref().unwrap_or("anonymous"),
                client_ip: &self.client_ip,
                operation: "SYMLINK",
                file_path: &format!("{} -> {}", full_link.to_string_lossy(), full_target.to_string_lossy()),
                file_size: 0,
                protocol: "SFTP",
                success: true,
                message: "符号链接创建成功",
            });
            self.logger.lock().unwrap().client_action(
                "SFTP",
                &format!("Created symlink: {} -> {}", link_path, target),
                &self.client_ip,
                self.username.as_deref(),
                "SYMLINK",
            );
            Ok(build_status_packet(id, SSH_FX_OK, "OK", ""))
        } else {
            Ok(build_status_packet(id, SSH_FX_FAILURE, "Failed to create symlink", ""))
        }
    }

    async fn handle_lock(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32(data, 1);
        let handle_str = parse_string(data, 5)?;

        if self.sftp_version < 5 {
            return Ok(build_status_packet(id, SSH_FX_OP_UNSUPPORTED, "Lock requires SFTP v5+", ""));
        }

        let handle = self.handles.get_mut(&handle_str);
        match handle {
            Some(SftpFileHandle::File { path, file, locked, .. }) => {
                if *locked {
                    return Ok(build_status_packet(id, SSH_FX_OK, "Already locked", ""));
                }

                let std_file = file.try_clone().await?.into_std().await;
                match fs2::FileExt::lock_exclusive(&std_file) {
                    Ok(()) => {
                        *locked = true;
                        self.locked_files.insert(path.clone());
                        self.logger.lock().unwrap().client_action(
                            "SFTP",
                            &format!("Locked file: {:?}", path),
                            &self.client_ip,
                            self.username.as_deref(),
                            "LOCK",
                        );
                        Ok(build_status_packet(id, SSH_FX_OK, "OK", ""))
                    }
                    Err(_) => Ok(build_status_packet(id, SSH_FX_FAILURE, "Failed to lock file", "")),
                }
            }
            _ => Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid handle", "")),
        }
    }

    async fn handle_unlock(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32(data, 1);
        let handle_str = parse_string(data, 5)?;

        let handle = self.handles.get_mut(&handle_str);
        match handle {
            Some(SftpFileHandle::File { path, file, locked, .. }) => {
                if !*locked {
                    return Ok(build_status_packet(id, SSH_FX_OK, "Not locked", ""));
                }

                let std_file = file.try_clone().await?.into_std().await;
                match fs2::FileExt::unlock(&std_file) {
                    Ok(()) => {
                        *locked = false;
                        self.locked_files.remove(path);
                        self.logger.lock().unwrap().client_action(
                            "SFTP",
                            &format!("Unlocked file: {:?}", path),
                            &self.client_ip,
                            self.username.as_deref(),
                            "UNLOCK",
                        );
                        Ok(build_status_packet(id, SSH_FX_OK, "OK", ""))
                    }
                    Err(_) => Ok(build_status_packet(id, SSH_FX_FAILURE, "Failed to unlock file", "")),
                }
            }
            _ => Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid handle", "")),
        }
    }
}
