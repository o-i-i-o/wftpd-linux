use anyhow::Result;

use super::state::SftpState;
use super::packet::*;
use super::packet::SSH_FX_OK;
use super::packet::SSH_FX_NO_SUCH_FILE;
use super::packet::SSH_FX_PERMISSION_DENIED;
use super::packet::SSH_FX_FAILURE;
use super::packet::SSH_FX_OP_UNSUPPORTED;
use crate::core::file_logger::FileLogInfo;

impl SftpState {
    pub async fn handle_extended(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let id = parse_u32(data, 1);
        
        if let Ok(mut log) = self.logger.lock() {
            log.debug("SFTP", &format!("[EXTENDED] Received packet, len={}, id={}", data.len(), id));
        }
        
        let (ext_name, ext_len) = parse_string_checked(data, 5)?;
        let ext_total_len = 4 + ext_len;
        
        if let Ok(mut log) = self.logger.lock() {
            log.debug("SFTP", &format!("[EXTENDED] extension='{}', ext_len={}, ext_total_len={}", ext_name, ext_len, ext_total_len));
        }

        match ext_name.as_str() {
            "limits@openssh.com" => self.handle_limits(id).await,
            "statvfs@openssh.com" => self.handle_statvfs(id, data, ext_total_len).await,
            "md5sum@openssh.com" | "md5-hash@openssh.com" => self.handle_md5sum(id, data, ext_total_len).await,
            "sha256sum@openssh.com" | "sha256-hash@openssh.com" => self.handle_sha256sum(id, data, ext_total_len).await,
            "copy-file" => self.handle_copy_file(id, data, ext_total_len).await,
            "hardlink@openssh.com" => self.handle_hardlink(id, data, ext_total_len).await,
            "posix-rename@openssh.com" => self.handle_posix_rename(id, data, ext_total_len).await,
            _ => {
                if let Ok(mut log) = self.logger.lock() {
                    log.warning("SFTP", &format!("[EXTENDED] Unsupported extension: {}", ext_name));
                }
                Ok(build_status_packet(id, SSH_FX_OP_UNSUPPORTED, &format!("Unsupported extension: {}", ext_name), ""))
            }
        }
    }

    async fn handle_limits(&self, id: u32) -> Result<Vec<u8>> {
        let max_packet_size: u64 = 32768;
        let max_read_size: u64 = 32768;
        let max_write_size: u64 = 32768;
        let max_open_handles: u64 = 1000;
        let max_locks: u64 = 100;

        let mut payload = vec![201];
        payload.extend_from_slice(&id.to_be_bytes());
        payload.extend_from_slice(&max_packet_size.to_be_bytes());
        payload.extend_from_slice(&max_read_size.to_be_bytes());
        payload.extend_from_slice(&max_write_size.to_be_bytes());
        payload.extend_from_slice(&max_open_handles.to_be_bytes());
        payload.extend_from_slice(&max_locks.to_be_bytes());
        Ok(build_packet(&payload))
    }

    async fn handle_statvfs(&self, id: u32, data: &[u8], ext_offset: usize) -> Result<Vec<u8>> {
        let (path, _) = parse_string_checked(data, 5 + ext_offset)?;
        let full_path = match self.resolve_path(&path) {
            Ok(p) => p,
            Err(e) => {
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Path resolution failed: {}", e), ""));
            }
        };

        #[cfg(unix)]
        {
            match tokio::fs::metadata(full_path.parent().unwrap_or(&full_path)).await {
                Ok(_metadata) => {
                    let bsize: u64 = 4096;
                    let frsize: u64 = 4096;
                    let blocks: u64 = 1000000;
                    let bfree: u64 = 500000;
                    let bavail: u64 = 500000;
                    let files: u64 = 100000;
                    let ffree: u64 = 50000;
                    let favail: u64 = 50000;
                    let fsid: u64 = 1;
                    let flag: u64 = 0;
                    let namemax: u64 = 255;

                    let mut payload = vec![201];
                    payload.extend_from_slice(&id.to_be_bytes());
                    payload.extend_from_slice(&bsize.to_be_bytes());
                    payload.extend_from_slice(&frsize.to_be_bytes());
                    payload.extend_from_slice(&blocks.to_be_bytes());
                    payload.extend_from_slice(&bfree.to_be_bytes());
                    payload.extend_from_slice(&bavail.to_be_bytes());
                    payload.extend_from_slice(&files.to_be_bytes());
                    payload.extend_from_slice(&ffree.to_be_bytes());
                    payload.extend_from_slice(&favail.to_be_bytes());
                    payload.extend_from_slice(&fsid.to_be_bytes());
                    payload.extend_from_slice(&flag.to_be_bytes());
                    payload.extend_from_slice(&namemax.to_be_bytes());
                    Ok(build_packet(&payload))
                }
                Err(_) => Ok(build_status_packet(id, SSH_FX_NO_SUCH_FILE, "No such file", "")),
            }
        }

        #[cfg(not(unix))]
        {
            Ok(build_status_packet(id, SSH_FX_OP_UNSUPPORTED, "statvfs not supported on this platform", ""))
        }
    }

    async fn handle_md5sum(&self, id: u32, data: &[u8], ext_offset: usize) -> Result<Vec<u8>> {
        let path = parse_string(data, 5 + ext_offset)?;
        let full_path = match self.resolve_path(&path) {
            Ok(p) => p,
            Err(e) => {
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Path resolution failed: {}", e), ""));
            }
        };

        if !self.check_permission(|p| p.can_read) {
            return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, "Permission denied", ""));
        }

        match tokio::fs::File::open(&full_path).await {
            Ok(mut file) => {
                use md5::{Md5, Digest};
                use tokio::io::AsyncReadExt;
                let mut hasher = Md5::new();
                let mut buffer = [0u8; 8192];
                loop {
                    match file.read(&mut buffer).await {
                        Ok(0) => break,
                        Ok(n) => hasher.update(&buffer[..n]),
                        Err(_) => return Ok(build_status_packet(id, SSH_FX_FAILURE, "Read error", "")),
                    }
                }
                let hash = hasher.finalize();
                let hash_hex = hex::encode(hash);

                let mut payload = vec![201];
                payload.extend_from_slice(&id.to_be_bytes());
                payload.extend_from_slice(&(hash_hex.len() as u32).to_be_bytes());
                payload.extend_from_slice(hash_hex.as_bytes());
                Ok(build_packet(&payload))
            }
            Err(_) => Ok(build_status_packet(id, SSH_FX_NO_SUCH_FILE, "No such file", "")),
        }
    }

    async fn handle_sha256sum(&self, id: u32, data: &[u8], ext_offset: usize) -> Result<Vec<u8>> {
        let path = parse_string(data, 5 + ext_offset)?;
        let full_path = match self.resolve_path(&path) {
            Ok(p) => p,
            Err(e) => {
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Path resolution failed: {}", e), ""));
            }
        };

        if !self.check_permission(|p| p.can_read) {
            return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, "Permission denied", ""));
        }

        match tokio::fs::File::open(&full_path).await {
            Ok(mut file) => {
                use sha2::{Sha256, Digest};
                use tokio::io::AsyncReadExt;
                let mut hasher = Sha256::new();
                let mut buffer = [0u8; 8192];
                loop {
                    match file.read(&mut buffer).await {
                        Ok(0) => break,
                        Ok(n) => hasher.update(&buffer[..n]),
                        Err(_) => return Ok(build_status_packet(id, SSH_FX_FAILURE, "Read error", "")),
                    }
                }
                let hash = hasher.finalize();
                let hash_hex = hex::encode(hash);

                let mut payload = vec![201];
                payload.extend_from_slice(&id.to_be_bytes());
                payload.extend_from_slice(&(hash_hex.len() as u32).to_be_bytes());
                payload.extend_from_slice(hash_hex.as_bytes());
                Ok(build_packet(&payload))
            }
            Err(_) => Ok(build_status_packet(id, SSH_FX_NO_SUCH_FILE, "No such file", "")),
        }
    }

    async fn handle_copy_file(&mut self, id: u32, data: &[u8], ext_offset: usize) -> Result<Vec<u8>> {
        let (src_path, src_len) = parse_string_checked(data, 5 + ext_offset)?;
        let dst_pos = 5 + ext_offset + 4 + src_len;
        let (dst_path, _) = parse_string_checked(data, dst_pos)?;

        if !self.check_permission(|p| p.can_read && p.can_write) {
            return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, "Permission denied", ""));
        }

        let src_full = match self.resolve_path(&src_path) {
            Ok(p) => p,
            Err(e) => {
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Path resolution failed: {}", e), ""));
            }
        };
        let dst_full = match self.resolve_path(&dst_path) {
            Ok(p) => p,
            Err(e) => {
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Path resolution failed: {}", e), ""));
            }
        };

        match tokio::fs::copy(&src_full, &dst_full).await {
            Ok(size) => {
                self.file_logger.lock().unwrap().log(FileLogInfo {
                    username: self.username.as_deref().unwrap_or("anonymous"),
                    client_ip: &self.client_ip,
                    operation: "COPY",
                    file_path: &format!("{} -> {}", src_full.to_string_lossy(), dst_full.to_string_lossy()),
                    file_size: size,
                    protocol: "SFTP",
                    success: true,
                    message: "文件复制成功",
                });
                self.logger.lock().unwrap().client_action(
                    "SFTP",
                    &format!("Copied: {} -> {}", src_path, dst_path),
                    &self.client_ip,
                    self.username.as_deref(),
                    "COPY",
                );
                Ok(build_status_packet(id, SSH_FX_OK, "OK", ""))
            }
            Err(_) => Ok(build_status_packet(id, SSH_FX_FAILURE, "Failed to copy file", "")),
        }
    }

    async fn handle_hardlink(&mut self, id: u32, data: &[u8], ext_offset: usize) -> Result<Vec<u8>> {
        let (src_path, src_len) = parse_string_checked(data, 5 + ext_offset)?;
        let dst_pos = 5 + ext_offset + 4 + src_len;
        let (dst_path, _) = parse_string_checked(data, dst_pos)?;

        if !self.check_permission(|p| p.can_write) {
            return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, "Permission denied", ""));
        }

        let src_full = match self.resolve_path(&src_path) {
            Ok(p) => p,
            Err(e) => {
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Path resolution failed: {}", e), ""));
            }
        };
        let dst_full = match self.resolve_path(&dst_path) {
            Ok(p) => p,
            Err(e) => {
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Path resolution failed: {}", e), ""));
            }
        };

        #[cfg(unix)]
        {
            match tokio::fs::hard_link(&src_full, &dst_full).await {
                Ok(_) => {
                    self.file_logger.lock().unwrap().log(FileLogInfo {
                        username: self.username.as_deref().unwrap_or("anonymous"),
                        client_ip: &self.client_ip,
                        operation: "HARDLINK",
                        file_path: &format!("{} -> {}", src_full.to_string_lossy(), dst_full.to_string_lossy()),
                        file_size: 0,
                        protocol: "SFTP",
                        success: true,
                        message: "硬链接创建成功",
                    });
                    self.logger.lock().unwrap().client_action(
                        "SFTP",
                        &format!("Hardlink: {} -> {}", src_path, dst_path),
                        &self.client_ip,
                        self.username.as_deref(),
                        "HARDLINK",
                    );
                    Ok(build_status_packet(id, SSH_FX_OK, "OK", ""))
                }
                Err(_) => Ok(build_status_packet(id, SSH_FX_FAILURE, "Failed to create hardlink", "")),
            }
        }

        #[cfg(not(unix))]
        {
            Ok(build_status_packet(id, SSH_FX_OP_UNSUPPORTED, "Hardlinks not supported on this platform", ""))
        }
    }

    async fn handle_posix_rename(&mut self, id: u32, data: &[u8], ext_offset: usize) -> Result<Vec<u8>> {
        let parse_offset = 5 + ext_offset;
        
        if let Ok(mut log) = self.logger.lock() {
            log.debug("SFTP", &format!("[POSIX-RENAME] data len={}, ext_offset={}, parse_offset={}", data.len(), ext_offset, parse_offset));
        }
        
        if parse_offset + 4 > data.len() {
            if let Ok(mut log) = self.logger.lock() {
                log.warning("SFTP", &format!("[POSIX-RENAME] Insufficient data for old_path length at offset {}", parse_offset));
            }
            return Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid packet", ""));
        }
        
        let (old_path, old_len) = parse_string_checked(data, parse_offset)?;
        let new_path_pos = parse_offset + 4 + old_len;
        
        if let Ok(mut log) = self.logger.lock() {
            log.debug("SFTP", &format!("[POSIX-RENAME] old_path='{}', old_len={}, new_path_pos={}", old_path, old_len, new_path_pos));
        }
        
        if new_path_pos + 4 > data.len() {
            if let Ok(mut log) = self.logger.lock() {
                log.warning("SFTP", &format!("[POSIX-RENAME] Insufficient data for new_path length at offset {}", new_path_pos));
            }
            return Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid packet", ""));
        }
        
        let (new_path, _) = parse_string_checked(data, new_path_pos)?;

        if let Ok(mut log) = self.logger.lock() {
            log.debug("SFTP", &format!("[POSIX-RENAME] new_path='{}'", new_path));
        }

        if !self.check_permission(|p| p.can_rename) {
            if let Ok(mut log) = self.logger.lock() {
                log.warning("SFTP", "[POSIX-RENAME] Permission denied");
            }
            return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, "Permission denied", ""));
        }

        let old_full = match self.resolve_path(&old_path) {
            Ok(p) => p,
            Err(e) => {
                if let Ok(mut log) = self.logger.lock() {
                    log.warning("SFTP", &format!("[POSIX-RENAME] Old path resolution failed: {}", e));
                }
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Path resolution failed: {}", e), ""));
            }
        };
        
        if let Ok(mut log) = self.logger.lock() {
            log.debug("SFTP", &format!("[POSIX-RENAME_OLD] path='{}', exists={}", old_full.display(), old_full.exists()));
        }
        
        if !old_full.exists() {
            if let Ok(mut log) = self.logger.lock() {
                log.warning("SFTP", &format!("[POSIX-RENAME] Old file not found: {}", old_full.display()));
            }
            return Ok(build_status_packet(id, SSH_FX_NO_SUCH_FILE, "No such file", ""));
        }
        
        let new_full = match self.resolve_path(&new_path) {
            Ok(p) => p,
            Err(e) => {
                if let Ok(mut log) = self.logger.lock() {
                    log.warning("SFTP", &format!("[POSIX-RENAME] New path resolution failed: {}", e));
                }
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Path resolution failed: {}", e), ""));
            }
        };
        
        if let Ok(mut log) = self.logger.lock() {
            log.debug("SFTP", &format!("[POSIX-RENAME_NEW] path='{}'", new_full.display()));
        }

        if let Ok(mut log) = self.logger.lock() {
            log.debug("SFTP", &format!("[POSIX-RENAME] Attempting rename: '{}' -> '{}'", old_full.display(), new_full.display()));
        }

        match tokio::fs::rename(&old_full, &new_full).await {
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
                        &format!("Renamed (posix): {} -> {}", old_path, new_path),
                        &self.client_ip,
                        self.username.as_deref(),
                        "RENAME",
                    );
                }
                if let Ok(mut log) = self.logger.lock() {
                    log.debug("SFTP", "[POSIX-RENAME] Success, returning SSH_FX_OK");
                }
                Ok(build_status_packet(id, SSH_FX_OK, "OK", ""))
            }
            Err(e) => {
                if let Ok(mut log) = self.logger.lock() {
                    log.warning("SFTP", &format!("[POSIX-RENAME] Failed: {}", e));
                }
                Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Failed to rename: {}", e), ""))
            }
        }
    }
}
