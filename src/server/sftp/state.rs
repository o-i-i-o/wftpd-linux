use anyhow::Result;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use super::packet::*;
use crate::core::logger::Logger;
use crate::core::users::UserManager;
use crate::core::file_logger::FileLogger;
use crate::server::common::utils::safe_resolve_path;

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

impl SftpState {
    pub async fn process_sftp_data(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        self.buffer.extend_from_slice(data);
        
        while self.buffer.len() >= 4 {
            let packet_len = u32::from_be_bytes([
                self.buffer[0], self.buffer[1], self.buffer[2], self.buffer[3]
            ]) as usize;
            
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
        let users = self.user_manager.lock().unwrap();
        if let Some(username) = &self.username {
            if let Some(user) = users.get_user(username) {
                return check_fn(&user.permissions);
            }
        }
        false
    }

    async fn handle_sftp_packet(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        if data.is_empty() {
            return Ok(build_status_packet(0, 4, "Bad packet", ""));
        }

        let msg_type = data[0];

        match msg_type {
            1 => self.handle_init(data).await,
            3 => self.handle_open(data).await,
            4 => self.handle_close(data).await,
            5 => self.handle_read(data).await,
            6 => self.handle_write(data).await,
            7 => self.handle_lstat(data).await,
            8 => self.handle_fstat(data).await,
            9 => self.handle_mkdir(data).await,
            10 => self.handle_rmdir(data).await,
            11 => self.handle_opendir(data).await,
            12 => self.handle_readdir(data).await,
            13 => self.handle_remove(data).await,
            14 => self.handle_mkdir(data).await,
            15 => self.handle_rmdir(data).await,
            16 => self.handle_realpath(data).await,
            17 => self.handle_readlink(data).await,
            18 => self.handle_symlink(data).await,
            19 => self.handle_rename(data).await,
            20 => self.handle_stat(data).await,
            22 => self.handle_remove(data).await,
            40 => self.handle_lock(data).await,
            41 => self.handle_unlock(data).await,
            200 => self.handle_extended(data).await,
            _ => Ok(build_status_packet(0, 8, "Unsupported operation", "")),
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
            return Ok(build_status_packet(id, 3, "Permission denied", ""));
        }

        let full_path = self.resolve_path(&path);

        if !full_path.exists() {
            return Ok(build_status_packet(id, 2, "No such directory", ""));
        }

        if !full_path.is_dir() {
            return Ok(build_status_packet(id, 4, "Not a directory", ""));
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
        Ok(build_status_packet(id, 0, "OK", ""))
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
                        return Ok(build_status_packet(id, 1, "End of directory", ""));
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
            None => Ok(build_status_packet(id, 4, "Invalid handle", "")),
        }
    }

    async fn handle_read(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32(data, 1);
        let handle_str = parse_string(data, 5)?;
        let offset = parse_u64(data, 5 + 4 + handle_str.len());
        let len = parse_u32(data, 5 + 4 + handle_str.len() + 8) as usize;

        if !self.check_permission(|p| p.can_read) {
            return Ok(build_status_packet(id, 3, "Permission denied", ""));
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
            _ => Ok(build_status_packet(id, 4, "Invalid handle", "")),
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
                return Ok(build_status_packet(id, 3, "Permission denied (append)", ""));
            }
        } else {
            if !self.check_permission(|p| p.can_write) {
                return Ok(build_status_packet(id, 3, "Permission denied", ""));
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

                Ok(build_status_packet(id, 0, "OK", ""))
            }
            _ => Ok(build_status_packet(id, 4, "Invalid handle", "")),
        }
    }

    async fn handle_remove(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32(data, 1);
        let path = parse_string(data, 5)?;

        if !self.check_permission(|p| p.can_delete) {
            return Ok(build_status_packet(id, 3, "Permission denied", ""));
        }

        let full_path = self.resolve_path(&path);

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
                Ok(build_status_packet(id, 0, "OK", ""))
            }
            Err(e) => {
                let error_msg = format!("Failed to remove file: {}", e);
                Ok(build_status_packet(id, 4, &error_msg, ""))
            }
        }
    }

    async fn handle_mkdir(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32(data, 1);
        let path = parse_string(data, 5)?;

        if !self.check_permission(|p| p.can_mkdir) {
            return Ok(build_status_packet(id, 3, "Permission denied", ""));
        }

        let full_path = self.resolve_path(&path);

        if tokio::fs::create_dir_all(&full_path).await.is_ok() {
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
            Ok(build_status_packet(id, 0, "OK", ""))
        } else {
            Ok(build_status_packet(id, 4, "Failed to create directory", ""))
        }
    }

    async fn handle_rmdir(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32(data, 1);
        let path = parse_string(data, 5)?;

        if !self.check_permission(|p| p.can_rmdir) {
            return Ok(build_status_packet(id, 3, "Permission denied", ""));
        }

        let full_path = self.resolve_path(&path);

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
            Ok(build_status_packet(id, 0, "OK", ""))
        } else {
            Ok(build_status_packet(id, 4, "Failed to remove directory", ""))
        }
    }

    async fn handle_rename(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32(data, 1);
        let old_path = parse_string(data, 5)?;
        let new_path_pos = 5 + 4 + old_path.len();
        let new_path = parse_string(data, new_path_pos)?;

        if !self.check_permission(|p| p.can_rename) {
            return Ok(build_status_packet(id, 3, "Permission denied", ""));
        }

        let old_full = self.resolve_path(&old_path);
        let new_full = self.resolve_path(&new_path);

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
            Ok(build_status_packet(id, 0, "OK", ""))
        } else {
            Ok(build_status_packet(id, 4, "Failed to rename", ""))
        }
    }

    async fn handle_stat(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32(data, 1);
        let path = parse_string(data, 5)?;

        if !self.check_permission(|p| p.can_read) {
            return Ok(build_status_packet(id, 3, "Permission denied", ""));
        }

        let full_path = self.resolve_path(&path);

        match tokio::fs::metadata(&full_path).await {
            Ok(metadata) => {
                let mut payload = vec![105];
                payload.extend_from_slice(&id.to_be_bytes());
                payload.extend_from_slice(&build_attrs(metadata.is_dir(), metadata.len()));
                Ok(build_packet(&payload))
            }
            Err(_) => Ok(build_status_packet(id, 2, "No such file", "")),
        }
    }

    async fn handle_lstat(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        self.handle_stat(data).await
    }

    async fn handle_fstat(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32(data, 1);
        let handle_str = parse_string(data, 5)?;

        if !self.check_permission(|p| p.can_read) {
            return Ok(build_status_packet(id, 3, "Permission denied", ""));
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
                    Err(_) => Ok(build_status_packet(id, 2, "No such file", "")),
                }
            }
            _ => Ok(build_status_packet(id, 4, "Invalid handle", "")),
        }
    }

    async fn handle_realpath(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32(data, 1);
        let path = parse_string(data, 5)?;

        if !self.check_permission(|p| p.can_read) {
            return Ok(build_status_packet(id, 3, "Permission denied", ""));
        }

        let full_path = self.resolve_path(&path);

        let resolved = if full_path.exists() {
            full_path.canonicalize().unwrap_or(full_path)
        } else {
            full_path
        };

        let path_str = resolved.to_string_lossy().to_string();

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

    pub fn resolve_path(&self, path: &str) -> PathBuf {
        safe_resolve_path(&self.home_dir, path)
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
        let id = parse_u32(data, 1);
        let path = parse_string(data, 5)?;
        let pflags_pos = 5 + 4 + path.len();
        let pflags = parse_u32(data, pflags_pos);

        let need_read = pflags & 0x00000001 != 0;
        let need_write = pflags & 0x00000002 != 0;
        let need_append = pflags & 0x00000008 != 0;

        if !self.check_permission(|p| {
            (!need_read || p.can_read) &&
            (!need_write || p.can_write) &&
            (!need_append || p.can_append)
        }) {
            return Ok(build_status_packet(id, 3, "Permission denied", ""));
        }

        let full_path = self.resolve_path(&path);
        let file_existed = full_path.exists();

        let file_result = if pflags & 0x00000002 != 0 {
            if pflags & 0x00000010 != 0 {
                tokio::fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(&full_path).await
            } else if pflags & 0x00000008 != 0 {
                tokio::fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .append(true)
                    .open(&full_path).await
            } else {
                tokio::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(false)
                    .open(&full_path).await
            }
        } else {
            tokio::fs::File::open(&full_path).await
        };

        match file_result {
            Ok(file) => {
                let handle = self.generate_handle();
                self.handles.insert(handle.clone(), SftpFileHandle::File {
                    path: full_path,
                    file,
                    locked: false,
                    existed: file_existed,
                });
                Ok(build_handle_packet(id, &handle))
            }
            Err(_) => Ok(build_status_packet(id, 4, "Failed to open file", "")),
        }
    }

    async fn handle_readlink(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32(data, 1);
        let path = parse_string(data, 5)?;

        let full_path = self.resolve_path(&path);

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
            Err(_) => Ok(build_status_packet(id, 2, "No such file", "")),
        }
    }

    async fn handle_symlink(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32(data, 1);
        let target = parse_string(data, 5)?;
        let link_pos = 5 + 4 + target.len();
        let link_path = parse_string(data, link_pos)?;

        if !self.check_permission(|p| p.can_write) {
            return Ok(build_status_packet(id, 3, "Permission denied", ""));
        }

        let full_link = self.resolve_path(&link_path);
        let home = PathBuf::from(&self.home_dir);
        let home_canon = home.canonicalize().unwrap_or(home);
        
        let full_target = if target.starts_with('/') {
            let resolved = PathBuf::from(&target);
            if resolved.exists() {
                match resolved.canonicalize() {
                    Ok(canon) if canon.starts_with(&home_canon) => canon,
                    _ => {
                        self.logger.lock().unwrap().client_action(
                            "SFTP",
                            "Symlink rejected: target outside home directory",
                            &self.client_ip,
                            self.username.as_deref(),
                            "SYMLINK_DENIED",
                        );
                        return Ok(build_status_packet(id, 3, "Permission denied: target outside home directory", ""));
                    }
                }
            } else {
                let mut safe_target = home_canon.clone();
                for component in resolved.components() {
                    match component {
                        std::path::Component::Normal(name) => {
                            safe_target.push(name);
                        }
                        std::path::Component::ParentDir => {
                            safe_target.pop();
                        }
                        _ => {}
                    }
                }
                if safe_target.starts_with(&home_canon) {
                    safe_target
                } else {
                    return Ok(build_status_packet(id, 3, "Permission denied: target outside home directory", ""));
                }
            }
        } else {
            self.resolve_path(&target)
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
            Ok(build_status_packet(id, 0, "OK", ""))
        } else {
            Ok(build_status_packet(id, 4, "Failed to create symlink", ""))
        }
    }

    async fn handle_lock(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32(data, 1);
        let handle_str = parse_string(data, 5)?;

        if self.sftp_version < 5 {
            return Ok(build_status_packet(id, 8, "Lock requires SFTP v5+", ""));
        }

        let handle = self.handles.get_mut(&handle_str);
        match handle {
            Some(SftpFileHandle::File { path, file, locked, .. }) => {
                if *locked {
                    return Ok(build_status_packet(id, 0, "Already locked", ""));
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
                        Ok(build_status_packet(id, 0, "OK", ""))
                    }
                    Err(_) => Ok(build_status_packet(id, 4, "Failed to lock file", "")),
                }
            }
            _ => Ok(build_status_packet(id, 4, "Invalid handle", "")),
        }
    }

    async fn handle_unlock(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32(data, 1);
        let handle_str = parse_string(data, 5)?;

        let handle = self.handles.get_mut(&handle_str);
        match handle {
            Some(SftpFileHandle::File { path, file, locked, .. }) => {
                if !*locked {
                    return Ok(build_status_packet(id, 0, "Not locked", ""));
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
                        Ok(build_status_packet(id, 0, "OK", ""))
                    }
                    Err(_) => Ok(build_status_packet(id, 4, "Failed to unlock file", "")),
                }
            }
            _ => Ok(build_status_packet(id, 4, "Invalid handle", "")),
        }
    }
}
