use anyhow::Result;
use std::path::Path;
use tracing::warn;
use nix::sys::statvfs::Statvfs;

use super::state::SftpState;
use super::packet::*;
use super::packet::SSH_FX_OK;
use super::packet::SSH_FX_NO_SUCH_FILE;
use super::packet::SSH_FX_PERMISSION_DENIED;
use super::packet::SSH_FX_FAILURE;
use super::packet::SSH_FX_OP_UNSUPPORTED;
use crate::core::file_logger::FileLogInfo;

fn io_error_to_sftp_status(e: &std::io::Error) -> (u32, &'static str) {
    match e.kind() {
        std::io::ErrorKind::NotFound => (SSH_FX_NO_SUCH_FILE, "No such file or directory"),
        std::io::ErrorKind::PermissionDenied => (SSH_FX_PERMISSION_DENIED, "Permission denied"),
        std::io::ErrorKind::AlreadyExists => (SSH_FX_FAILURE, "File already exists"),
        std::io::ErrorKind::IsADirectory => (SSH_FX_FAILURE, "Is a directory"),
        std::io::ErrorKind::NotADirectory => (SSH_FX_FAILURE, "Not a directory"),
        _ => {
            let raw_os_error = e.raw_os_error().unwrap_or(0);
            match raw_os_error {
                libc::EXDEV => (SSH_FX_FAILURE, "Cross-device link not supported"),
                libc::ENOSPC => (SSH_FX_FAILURE, "No space left on device"),
                libc::EROFS => (SSH_FX_PERMISSION_DENIED, "Read-only file system"),
                libc::EACCES => (SSH_FX_PERMISSION_DENIED, "Permission denied"),
                libc::ENOENT => (SSH_FX_NO_SUCH_FILE, "No such file or directory"),
                libc::ENOTDIR => (SSH_FX_FAILURE, "Not a directory"),
                libc::EISDIR => (SSH_FX_FAILURE, "Is a directory"),
                libc::EEXIST => (SSH_FX_FAILURE, "File already exists"),
                libc::EMLINK => (SSH_FX_FAILURE, "Too many links"),
                _ => (SSH_FX_FAILURE, "Operation failed"),
            }
        }
    }
}

fn log_io_error(operation: &str, path: &Path, error: &std::io::Error) {
    warn!(operation = operation, path = %path.display(), error = %error, "[SFTP] Operation failed");
}

fn build_statvfs_reply(id: u32, stats: &Statvfs) -> Vec<u8> {
    let bsize: u64 = stats.block_size();
    let frsize: u64 = stats.fragment_size();
    let blocks: u64 = stats.blocks();
    let bfree: u64 = stats.blocks_free();
    let bavail: u64 = stats.blocks_available();
    let files: u64 = stats.files();
    let ffree: u64 = stats.files_free();
    let favail: u64 = stats.files_available();
    let fsid: u64 = stats.filesystem_id();
    let namemax: u64 = stats.name_max();

    let mut flag: u64 = 0;
    use nix::sys::statvfs::FsFlags;
    if stats.flags().contains(FsFlags::ST_RDONLY) {
        flag |= 1;
    }

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
    build_packet(&payload)
}

impl SftpState {
    pub async fn handle_extended(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        if data.len() < 9 {
            return Ok(build_status_packet(0, SSH_FX_FAILURE, "Invalid extended packet: too short", ""));
        }
        
        let id = parse_u32(data, 1);
        
        let ext_name_len_pos = 5;
        if ext_name_len_pos + 4 > data.len() {
            return Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid extended packet: extension name length overflow", ""));
        }
        
        let (ext_name, ext_len) = match parse_string_checked(data, ext_name_len_pos) {
            Ok(result) => result,
            Err(e) => {
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Invalid extension name: {}", e), ""));
            }
        };
        
        let ext_total_len = 4usize.checked_add(ext_len)
            .ok_or_else(|| anyhow::anyhow!("Extension length overflow"))?;
        
        let remaining_data = data.len().saturating_sub(5 + ext_total_len);
        if remaining_data < 4 && ext_name != "limits@openssh.com" {
            return Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid extended packet: missing data", ""));
        }

        match ext_name.as_str() {
            "limits@openssh.com" => self.handle_limits(id).await,
            "statvfs@openssh.com" => self.handle_statvfs(id, data, ext_total_len).await,
            "fstatvfs@openssh.com" => self.handle_fstatvfs(id, data, ext_total_len).await,
            "fsync@openssh.com" => self.handle_fsync(id, data, ext_total_len).await,
            "md5sum@openssh.com" | "md5-hash@openssh.com" => self.handle_md5sum(id, data, ext_total_len).await,
            "sha256sum@openssh.com" | "sha256-hash@openssh.com" => self.handle_sha256sum(id, data, ext_total_len).await,
            "copy-file" => self.handle_copy_file(id, data, ext_total_len).await,
            "hardlink@openssh.com" => self.handle_hardlink(id, data, ext_total_len).await,
            "posix-rename@openssh.com" => self.handle_posix_rename(id, data, ext_total_len).await,
            _ => {
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
        let parse_offset = 5usize.checked_add(ext_offset)
            .ok_or_else(|| anyhow::anyhow!("Offset overflow"))?;
        
        if parse_offset + 4 > data.len() {
            return Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid statvfs packet: missing path length", ""));
        }
        
        let (path, path_len) = match parse_string_checked(data, parse_offset) {
            Ok(result) => result,
            Err(e) => {
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Invalid path: {}", e), ""));
            }
        };
        
        let expected_end = parse_offset.checked_add(4 + path_len);
        if let Some(end) = expected_end
            && end > data.len() {
                return Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid statvfs packet: path data truncated", ""));
            }
        
        let full_path = match self.resolve_path(&path) {
            Ok(p) => p,
            Err(e) => {
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Path resolution failed: {}", e), ""));
            }
        };

        use nix::sys::statvfs::statvfs;
        match statvfs(&full_path) {
            Ok(stats) => Ok(build_statvfs_reply(id, &stats)),
            Err(e) => {
                let (status, msg) = match e {
                    nix::errno::Errno::ENOENT => (SSH_FX_NO_SUCH_FILE, "No such file or directory"),
                    nix::errno::Errno::EACCES => (SSH_FX_PERMISSION_DENIED, "Permission denied"),
                    nix::errno::Errno::ENOTDIR => (SSH_FX_FAILURE, "Not a directory"),
                    _ => (SSH_FX_FAILURE, "Failed to get filesystem stats"),
                };
                Ok(build_status_packet(id, status, msg, ""))
            }
        }
    }

    async fn handle_fstatvfs(&self, id: u32, data: &[u8], ext_offset: usize) -> Result<Vec<u8>> {
        let parse_offset = 5usize.checked_add(ext_offset)
            .ok_or_else(|| anyhow::anyhow!("Offset overflow"))?;

        if parse_offset + 4 > data.len() {
            return Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid fstatvfs packet: missing handle length", ""));
        }

        let (handle_str, handle_len) = match parse_string_checked(data, parse_offset) {
            Ok(result) => result,
            Err(e) => {
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Invalid handle: {}", e), ""));
            }
        };

        let expected_end = parse_offset.checked_add(4 + handle_len);
        if let Some(end) = expected_end
            && end > data.len() {
                return Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid fstatvfs packet: handle data truncated", ""));
            }

        use nix::sys::statvfs::fstatvfs;

        match self.handles.get(&handle_str) {
            Some(handle) if !handle.is_dir => {
                match fstatvfs(&handle.file) {
                    Ok(stats) => Ok(build_statvfs_reply(id, &stats)),
                    Err(e) => {
                        let (status, msg) = match e {
                            nix::errno::Errno::ENOENT => (SSH_FX_NO_SUCH_FILE, "No such file or directory"),
                            nix::errno::Errno::EACCES => (SSH_FX_PERMISSION_DENIED, "Permission denied"),
                            nix::errno::Errno::ENOTDIR => (SSH_FX_FAILURE, "Not a directory"),
                            _ => (SSH_FX_FAILURE, "Failed to get filesystem stats"),
                        };
                        Ok(build_status_packet(id, status, msg, ""))
                    }
                }
            }
            Some(_) => Ok(build_status_packet(id, SSH_FX_FAILURE, "Handle does not reference a file", "")),
            None => Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid handle", "")),
        }
    }

    async fn handle_fsync(&self, id: u32, data: &[u8], ext_offset: usize) -> Result<Vec<u8>> {
        let parse_offset = 5usize.checked_add(ext_offset)
            .ok_or_else(|| anyhow::anyhow!("Offset overflow"))?;

        if parse_offset + 4 > data.len() {
            return Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid fsync packet: missing handle length", ""));
        }

        let (handle_str, handle_len) = match parse_string_checked(data, parse_offset) {
            Ok(result) => result,
            Err(e) => {
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Invalid handle: {}", e), ""));
            }
        };

        let expected_end = parse_offset.checked_add(4 + handle_len);
        if let Some(end) = expected_end
            && end > data.len() {
                return Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid fsync packet: handle data truncated", ""));
            }

        match self.handles.get(&handle_str) {
            Some(handle) if !handle.is_dir => match handle.file.sync_all().await {
                Ok(_) => Ok(build_status_packet(id, SSH_FX_OK, "OK", "")),
                Err(e) => {
                    log_io_error("FSYNC", &handle.path, &e);
                    let (status, msg) = io_error_to_sftp_status(&e);
                    Ok(build_status_packet(id, status, &format!("{}: {}", msg, e), ""))
                }
            },
            Some(_) => Ok(build_status_packet(id, SSH_FX_FAILURE, "Handle does not reference a file", "")),
            None => Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid handle", "")),
        }
    }

    async fn handle_md5sum(&self, id: u32, data: &[u8], ext_offset: usize) -> Result<Vec<u8>> {
        let parse_offset = 5usize.checked_add(ext_offset)
            .ok_or_else(|| anyhow::anyhow!("Offset overflow"))?;
        
        if parse_offset + 4 > data.len() {
            return Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid md5sum packet: missing path length", ""));
        }
        
        let (path, path_len) = match parse_string_checked(data, parse_offset) {
            Ok(result) => result,
            Err(e) => {
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Invalid path: {}", e), ""));
            }
        };
        
        let expected_end = parse_offset.checked_add(4 + path_len);
        if let Some(end) = expected_end
            && end > data.len() {
                return Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid md5sum packet: path data truncated", ""));
            }
        
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
                        Err(e) => {
                            let (status, msg) = io_error_to_sftp_status(&e);
                            return Ok(build_status_packet(id, status, msg, ""));
                        }
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
            Err(e) => {
                let (status, msg) = io_error_to_sftp_status(&e);
                Ok(build_status_packet(id, status, msg, ""))
            }
        }
    }

    async fn handle_sha256sum(&self, id: u32, data: &[u8], ext_offset: usize) -> Result<Vec<u8>> {
        let parse_offset = 5usize.checked_add(ext_offset)
            .ok_or_else(|| anyhow::anyhow!("Offset overflow"))?;
        
        if parse_offset + 4 > data.len() {
            return Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid sha256sum packet: missing path length", ""));
        }
        
        let (path, path_len) = match parse_string_checked(data, parse_offset) {
            Ok(result) => result,
            Err(e) => {
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Invalid path: {}", e), ""));
            }
        };
        
        let expected_end = parse_offset.checked_add(4 + path_len);
        if let Some(end) = expected_end
            && end > data.len() {
                return Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid sha256sum packet: path data truncated", ""));
            }
        
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
                        Err(e) => {
                            let (status, msg) = io_error_to_sftp_status(&e);
                            return Ok(build_status_packet(id, status, msg, ""));
                        }
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
            Err(e) => {
                let (status, msg) = io_error_to_sftp_status(&e);
                Ok(build_status_packet(id, status, msg, ""))
            }
        }
    }

    async fn handle_copy_file(&mut self, id: u32, data: &[u8], ext_offset: usize) -> Result<Vec<u8>> {
        let parse_offset = 5usize.checked_add(ext_offset)
            .ok_or_else(|| anyhow::anyhow!("Offset overflow"))?;
        
        if parse_offset + 4 > data.len() {
            return Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid copy-file packet: missing source path length", ""));
        }
        
        let (src_path, src_len) = match parse_string_checked(data, parse_offset) {
            Ok(result) => result,
            Err(e) => {
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Invalid source path: {}", e), ""));
            }
        };
        
        let src_end = parse_offset.checked_add(4 + src_len)
            .ok_or_else(|| anyhow::anyhow!("Source path end overflow"))?;
        if src_end > data.len() {
            return Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid copy-file packet: source path data truncated", ""));
        }
        
        let dst_pos = src_end;
        if dst_pos + 4 > data.len() {
            return Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid copy-file packet: missing destination path length", ""));
        }
        
        let (dst_path, dst_len) = match parse_string_checked(data, dst_pos) {
            Ok(result) => result,
            Err(e) => {
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Invalid destination path: {}", e), ""));
            }
        };
        
        let dst_end = dst_pos.checked_add(4 + dst_len)
            .ok_or_else(|| anyhow::anyhow!("Destination path end overflow"))?;
        if dst_end > data.len() {
            return Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid copy-file packet: destination path data truncated", ""));
        }

        if !self.check_permission(|p| p.can_read && p.can_write) {
            return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, "Permission denied", ""));
        }

        let src_full = match self.resolve_path(&src_path) {
            Ok(p) => p,
            Err(e) => {
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Source path resolution failed: {}", e), ""));
            }
        };
        let dst_full = match self.resolve_path(&dst_path) {
            Ok(p) => p,
            Err(e) => {
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Destination path resolution failed: {}", e), ""));
            }
        };

        if src_full == dst_full {
            return Ok(build_status_packet(id, SSH_FX_FAILURE, "Source and destination are the same file", ""));
        }

        let src_metadata = match tokio::fs::metadata(&src_full).await {
            Ok(metadata) => metadata,
            Err(e) => {
                let (status, msg) = io_error_to_sftp_status(&e);
                return Ok(build_status_packet(id, status, &format!("{}: {}", msg, e), ""));
            }
        };

        if !src_metadata.is_file() {
            let msg = if src_metadata.is_dir() {
                "Source is a directory, not a file"
            } else {
                "Source is not a regular file"
            };
            return Ok(build_status_packet(id, SSH_FX_FAILURE, msg, ""));
        }

        let dst_parent = match dst_full.parent() {
            Some(parent) => parent,
            None => {
                return Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid destination path", ""));
            }
        };

        match tokio::fs::try_exists(dst_parent).await {
            Ok(true) => {}
            Ok(false) => {
                return Ok(build_status_packet(id, SSH_FX_NO_SUCH_FILE, "Destination directory does not exist", ""));
            }
            Err(e) => {
                let (status, msg) = io_error_to_sftp_status(&e);
                return Ok(build_status_packet(id, status, &format!("{}: {}", msg, e), ""));
            }
        }

        let overwrite = data.get(dst_end).copied().unwrap_or(0) != 0;
        let src_size = src_metadata.len();
        let additional_bytes = match tokio::fs::metadata(&dst_full).await {
            Ok(dst_metadata) => {
                if dst_metadata.is_dir() {
                    return Ok(build_status_packet(id, SSH_FX_FAILURE, "Destination is a directory", ""));
                }
                if !overwrite {
                    return Ok(build_status_packet(id, SSH_FX_FAILURE, "Destination file already exists", ""));
                }
                src_size.saturating_sub(dst_metadata.len())
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => src_size,
            Err(e) => {
                let (status, msg) = io_error_to_sftp_status(&e);
                return Ok(build_status_packet(id, status, &format!("{}: {}", msg, e), ""));
            }
        };

        if !self.check_quota_for_additional_bytes(additional_bytes) {
            return Ok(build_status_packet(id, SSH_FX_FAILURE, "Quota exceeded", ""));
        }

        match tokio::fs::copy(&src_full, &dst_full).await {
            Ok(size) => {
                if let Ok(mut fl) = self.file_logger.try_lock() {
                    fl.log(FileLogInfo {
                        username: self.username.as_deref().unwrap_or("anonymous"),
                        client_ip: &self.client_ip,
                        operation: "COPY",
                        file_path: &format!("{} -> {}", src_full.to_string_lossy(), dst_full.to_string_lossy()),
                        file_size: size,
                        protocol: "SFTP",
                        success: true,
                        message: "文件复制成功",
                    });
                }
                self.invalidate_quota_cache();
                Ok(build_status_packet(id, SSH_FX_OK, "OK", ""))
            }
            Err(e) => {
                log_io_error("COPY", &src_full, &e);
                let (status, msg) = io_error_to_sftp_status(&e);
                Ok(build_status_packet(id, status, &format!("{}: {}", msg, e), ""))
            }
        }
    }

    async fn handle_hardlink(&mut self, id: u32, data: &[u8], ext_offset: usize) -> Result<Vec<u8>> {
        let parse_offset = 5usize.checked_add(ext_offset)
            .ok_or_else(|| anyhow::anyhow!("Offset overflow"))?;
        
        if parse_offset + 4 > data.len() {
            return Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid hardlink packet: missing source path length", ""));
        }
        
        let (src_path, src_len) = match parse_string_checked(data, parse_offset) {
            Ok(result) => result,
            Err(e) => {
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Invalid source path: {}", e), ""));
            }
        };
        
        let src_end = parse_offset.checked_add(4 + src_len)
            .ok_or_else(|| anyhow::anyhow!("Source path end overflow"))?;
        if src_end > data.len() {
            return Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid hardlink packet: source path data truncated", ""));
        }
        
        let dst_pos = src_end;
        if dst_pos + 4 > data.len() {
            return Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid hardlink packet: missing destination path length", ""));
        }
        
        let (dst_path, dst_len) = match parse_string_checked(data, dst_pos) {
            Ok(result) => result,
            Err(e) => {
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Invalid destination path: {}", e), ""));
            }
        };
        
        let dst_end = dst_pos.checked_add(4 + dst_len)
            .ok_or_else(|| anyhow::anyhow!("Destination path end overflow"))?;
        if dst_end > data.len() {
            return Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid hardlink packet: destination path data truncated", ""));
        }

        if !self.check_permission(|p| p.can_write) {
            return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, "Permission denied", ""));
        }

        let src_full = match self.resolve_path(&src_path) {
            Ok(p) => p,
            Err(e) => {
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Source path resolution failed: {}", e), ""));
            }
        };
        let dst_full = match self.resolve_path(&dst_path) {
            Ok(p) => p,
            Err(e) => {
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Destination path resolution failed: {}", e), ""));
            }
        };

        match tokio::fs::hard_link(&src_full, &dst_full).await {
            Ok(_) => {
                if let Ok(mut fl) = self.file_logger.try_lock() {
                    fl.log(FileLogInfo {
                        username: self.username.as_deref().unwrap_or("anonymous"),
                        client_ip: &self.client_ip,
                        operation: "HARDLINK",
                        file_path: &format!("{} -> {}", src_full.to_string_lossy(), dst_full.to_string_lossy()),
                        file_size: 0,
                        protocol: "SFTP",
                        success: true,
                        message: "硬链接创建成功",
                    });
                }
                Ok(build_status_packet(id, SSH_FX_OK, "OK", ""))
            }
            Err(e) => {
                log_io_error("HARDLINK", &src_full, &e);
                let (status, msg) = io_error_to_sftp_status(&e);
                Ok(build_status_packet(id, status, &format!("{}: {}", msg, e), ""))
            }
        }
    }

    async fn handle_posix_rename(&mut self, id: u32, data: &[u8], ext_offset: usize) -> Result<Vec<u8>> {
        let parse_offset = 5usize.checked_add(ext_offset)
            .ok_or_else(|| anyhow::anyhow!("Offset overflow"))?;
        
        if parse_offset + 4 > data.len() {
            return Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid posix-rename packet: missing old path length", ""));
        }
        
        let (old_path, old_len) = match parse_string_checked(data, parse_offset) {
            Ok(result) => result,
            Err(e) => {
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Invalid old path: {}", e), ""));
            }
        };
        
        let old_end = parse_offset.checked_add(4 + old_len)
            .ok_or_else(|| anyhow::anyhow!("Old path end overflow"))?;
        if old_end > data.len() {
            return Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid posix-rename packet: old path data truncated", ""));
        }
        
        let new_path_pos = old_end;
        
        if new_path_pos + 4 > data.len() {
            return Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid posix-rename packet: missing new path length", ""));
        }
        
        let (new_path, new_len) = match parse_string_checked(data, new_path_pos) {
            Ok(result) => result,
            Err(e) => {
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Invalid new path: {}", e), ""));
            }
        };
        
        let new_end = new_path_pos.checked_add(4 + new_len)
            .ok_or_else(|| anyhow::anyhow!("New path end overflow"))?;
        if new_end > data.len() {
            return Ok(build_status_packet(id, SSH_FX_FAILURE, "Invalid posix-rename packet: new path data truncated", ""));
        }

        if !self.check_permission(|p| p.can_rename) {
            return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, "Permission denied", ""));
        }

        let old_full = match self.resolve_path(&old_path) {
            Ok(p) => p,
            Err(e) => {
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Old path resolution failed: {}", e), ""));
            }
        };

        let new_full = match self.resolve_path(&new_path) {
            Ok(p) => p,
            Err(e) => {
                return Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("New path resolution failed: {}", e), ""));
            }
        };

        match tokio::fs::rename(&old_full, &new_full).await {
            Ok(_) => {
                if let Ok(mut fl) = self.file_logger.try_lock() {
                    fl.log_rename(
                        self.username.as_deref().unwrap_or("anonymous"),
                        &self.client_ip,
                        &old_full.to_string_lossy(),
                        &new_full.to_string_lossy(),
                        "SFTP",
                    );
                }
                Ok(build_status_packet(id, SSH_FX_OK, "OK", ""))
            }
            Err(e) => {
                log_io_error("POSIX-RENAME", &old_full, &e);
                let (status, msg) = io_error_to_sftp_status(&e);
                Ok(build_status_packet(id, status, &format!("{}: {}", msg, e), ""))
            }
        }
    }
}
