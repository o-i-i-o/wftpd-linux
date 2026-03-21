use anyhow::Result;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::fs::File;

use super::super::data_connection::get_data_connection;
use super::super::handler::FtpSession;
use super::super::utils::{build_mlst_facts, get_file_mtime, safe_resolve_path, escape_mlst_filename};
use crate::core::file_logger::FileLogInfo;

impl FtpSession {
    pub async fn cmd_list(&mut self, _arg: Option<&str>) -> Result<()> {
        if !self.authenticated {
            self.stream.write_all(b"530 Not logged in\r\n").await?;
            return Ok(());
        }

        let can_list = {
            let users = self.user_manager.lock().unwrap();
            let user = self.current_user.as_ref().and_then(|u| users.get_user(u));
            user.is_none_or(|u| u.permissions.can_list)
        };
        
        if !can_list {
            self.stream.write_all(b"550 Permission denied\r\n").await?;
            return Ok(());
        }

        self.stream.write_all(b"150 Here comes the directory listing\r\n").await?;

        let cwd = self.cwd.clone();
        let data_timeout = self.get_data_timeout();
        let data_result = get_data_connection(
            self.passive_mode,
            self.data_port,
            &self.data_addr,
            &self.remote_ip,
            &self.passive_listeners,
            data_timeout,
        ).await;
        
        match data_result {
            Ok(mut data_stream) => {
                let path = Path::new(&cwd);
                match std::fs::read_dir(path) {
                    Ok(entries) => {
                        for entry in entries.flatten() {
                            if let Ok(metadata) = entry.metadata() {
                                let name = entry.file_name().to_string_lossy().to_string();
                                let perms = if metadata.is_dir() {
                                    "drwxr-xr-x"
                                } else {
                                    "-rw-r--r--"
                                };
                                let size = metadata.len();
                                let mtime = get_file_mtime(&metadata);
                                let line = format!(
                                    "{} 1 user user {:>10} {} {}\r\n",
                                    perms, size, mtime, name
                                );
                                let _ = data_stream.write_all(line.as_bytes()).await;
                            }
                        }
                    }
                    Err(e) => {
                        self.logger.lock().unwrap().warning(
                            "FTP",
                            &format!("Failed to read directory {}: {}", cwd, e),
                        );
                    }
                }
                self.cleanup_data_connection().await;
                self.stream.write_all(b"226 Transfer complete\r\n").await?;
            }
            Err(e) => {
                self.logger.lock().unwrap().warning(
                    "FTP",
                    &format!("Failed to get data connection: {}", e),
                );
                self.cleanup_data_connection().await;
                self.stream.write_all(b"425 Cannot open data connection\r\n").await?;
            }
        }

        Ok(())
    }

    pub async fn cmd_mlsd(&mut self) -> Result<()> {
        if !self.authenticated {
            self.stream.write_all(b"530 Not logged in\r\n").await?;
            return Ok(());
        }

        let can_list = {
            let users = self.user_manager.lock().unwrap();
            let user = self.current_user.as_ref().and_then(|u| users.get_user(u));
            user.is_none_or(|u| u.permissions.can_list)
        };
        
        if !can_list {
            self.stream.write_all(b"550 Permission denied\r\n").await?;
            return Ok(());
        }

        self.stream.write_all(b"150 Here comes the directory listing\r\n").await?;

        let data_timeout = self.get_data_timeout();
        let data_result = get_data_connection(
            self.passive_mode,
            self.data_port,
            &self.data_addr,
            &self.remote_ip,
            &self.passive_listeners,
            data_timeout,
        ).await;

        match data_result {
            Ok(mut data_stream) => {
                let path = Path::new(&self.cwd);
                if let Ok(entries) = std::fs::read_dir(path) {
                    for entry in entries.flatten() {
                        if let Ok(metadata) = entry.metadata() {
                            let name = entry.file_name().to_string_lossy().to_string();
                            let facts = build_mlst_facts(&metadata);
                            let escaped_name = escape_mlst_filename(&name);
                            let line = format!("{} {}\r\n", facts, escaped_name);
                            let _ = data_stream.write_all(line.as_bytes()).await;
                        }
                    }
                }
                self.cleanup_data_connection().await;
                self.stream.write_all(b"226 Transfer complete\r\n").await?;
            }
            Err(e) => {
                self.logger.lock().unwrap().warning(
                    "FTP",
                    &format!("Failed to get data connection: {}", e),
                );
                self.cleanup_data_connection().await;
                self.stream.write_all(b"425 Cannot open data connection\r\n").await?;
            }
        }
        Ok(())
    }

    pub async fn cmd_retr(&mut self, arg: Option<&str>) -> Result<()> {
        if !self.authenticated {
            self.stream.write_all(b"530 Not logged in\r\n").await?;
            return Ok(());
        }

        if let Some(filename) = arg {
            let file_path = match safe_resolve_path(&self.cwd, &self.home_dir, filename) {
                Ok(p) => p,
                Err(e) => {
                    let error_msg = format!("550 Path resolution failed: {}\r\n", e);
                    self.stream.write_all(error_msg.as_bytes()).await?;
                    return Ok(());
                }
            };
            
            self.logger.lock().unwrap().debug(
                "FTP",
                &format!(
                    "RETR: input='{}', cwd='{}', home='{}', resolved='{}'",
                    filename, self.cwd, self.home_dir, file_path.display()
                ),
            );

            if !file_path.exists() || !file_path.is_file() || !file_path.starts_with(Path::new(&self.home_dir)) {
                self.logger.lock().unwrap().warning(
                    "FTP",
                    &format!(
                        "RETR: File not found or access denied: {} (exists={}, is_file={}, in_home={})",
                        file_path.display(),
                        file_path.exists(),
                        file_path.is_file(),
                        file_path.starts_with(&self.home_dir)
                    ),
                );
                self.stream.write_all(b"550 File not found\r\n").await?;
                return Ok(());
            }

            let can_read = {
                let users = self.user_manager.lock().unwrap();
                let user = self.current_user.as_ref().and_then(|u| users.get_user(u));
                (
                    user.is_none_or(|u| u.permissions.can_read),
                    user.and_then(|u| u.permissions.speed_limit_kbps).unwrap_or(0),
                )
            };

            if !can_read.0 {
                self.stream.write_all(b"550 Permission denied\r\n").await?;
                return Ok(());
            }
            let user_speed_limit = can_read.1;

            let file_size = std::fs::metadata(&file_path)?.len();
            let remaining = if self.rest_offset > 0 && self.rest_offset < file_size {
                file_size - self.rest_offset
            } else {
                file_size
            };

            self.stream.write_all(
                format!("150 Opening BINARY mode data connection ({} bytes)\r\n", remaining)
                    .as_bytes(),
            ).await?;

            let data_timeout = self.get_data_timeout();
            let data_result = get_data_connection(
                self.passive_mode,
                self.data_port,
                &self.data_addr,
                &self.remote_ip,
                &self.passive_listeners,
                data_timeout,
            ).await;

            match data_result {
                Ok(mut data_stream) => {
                    let abort = Arc::clone(&self.abort_flag);
                    if let Ok(mut file) = File::open(&file_path).await {
                        if self.rest_offset > 0 {
                            let _ = file.seek(std::io::SeekFrom::Start(self.rest_offset)).await;
                        }

                        let mut buf = [0u8; 8192];
                        let mut transfer_success = true;
                        let mut total_read: u64 = 0;
                        let global_speed_limit = self.config.lock().unwrap().ftp.max_speed_kbps;
                        let effective_speed_limit = if user_speed_limit > 0 {
                            Some(user_speed_limit)
                        } else if global_speed_limit > 0 {
                            Some(global_speed_limit)
                        } else {
                            None
                        };
                        let mut last_throttle_time = Instant::now();
                        let mut bytes_since_throttle: u64 = 0;
                        
                        loop {
                            if abort.load(Ordering::Relaxed) {
                                transfer_success = false;
                                break;
                            }
                            match file.read(&mut buf).await {
                                Ok(0) => break,
                                Ok(n) => {
                                    total_read += n as u64;
                                    if data_stream.write_all(&buf[..n]).await.is_err() {
                                        log::error!("RETR write error to data stream for file: {}", file_path.display());
                                        transfer_success = false;
                                        break;
                                    }
                                    
                                    if let Some(limit_kbps) = effective_speed_limit {
                                        bytes_since_throttle += n as u64;
                                        let elapsed = last_throttle_time.elapsed();
                                        if elapsed >= Duration::from_millis(100) {
                                            let max_bytes = (limit_kbps * 1024) as f64 * elapsed.as_secs_f64();
                                            if bytes_since_throttle > max_bytes as u64 {
                                                let excess = bytes_since_throttle - max_bytes as u64;
                                                let wait_secs = excess as f64 / (limit_kbps * 1024) as f64;
                                                tokio::time::sleep(Duration::from_secs_f64(wait_secs)).await;
                                            }
                                            last_throttle_time = Instant::now();
                                            bytes_since_throttle = 0;
                                        }
                                    }
                                }
                                Err(e) => {
                                    log::error!("RETR read error from file: {} - {}", file_path.display(), e);
                                    transfer_success = false;
                                    break;
                                }
                            }
                        }
                        if transfer_success {
                            log::info!("RETR completed: {} bytes sent from {}", total_read, file_path.display());
                        }
                    } else {
                        log::error!("RETR failed to open file: {}", file_path.display());
                    }
                    self.cleanup_data_connection().await;
                    self.stream.write_all(b"226 Transfer complete\r\n").await?;
                }
                Err(e) => {
                    self.logger.lock().unwrap().warning(
                        "FTP",
                        &format!("Failed to get data connection: {}", e),
                    );
                    self.cleanup_data_connection().await;
                    self.stream.write_all(b"425 Cannot open data connection\r\n").await?;
                    return Ok(());
                }
            }

            let file_size = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(remaining);
            self.file_logger.lock().unwrap().log_download(
                self.current_user.as_deref().unwrap_or("anonymous"),
                &self.remote_ip,
                &file_path.to_string_lossy(),
                file_size,
                "FTP",
            );

            self.logger.lock().unwrap().client_action(
                "FTP",
                &format!(
                    "Downloaded: {} ({} bytes from offset {})",
                    filename, remaining, self.rest_offset
                ),
                &self.remote_ip,
                self.current_user.as_deref(),
                "DOWNLOAD",
            );

            self.rest_offset = 0;
        }
        Ok(())
    }

    pub async fn cmd_stor(&mut self, arg: Option<&str>) -> Result<()> {
        if !self.authenticated {
            self.stream.write_all(b"530 Not logged in\r\n").await?;
            return Ok(());
        }

        if let Some(filename) = arg {
            let (can_write, quota_mb, user_speed_limit) = {
                let users = self.user_manager.lock().unwrap();
                let user = self.current_user.as_ref().and_then(|u| users.get_user(u));
                (
                    user.is_none_or(|u| u.permissions.can_write),
                    user.and_then(|u| u.permissions.quota_mb).unwrap_or(0),
                    user.and_then(|u| u.permissions.speed_limit_kbps).unwrap_or(0),
                )
            };

            if !can_write {
                self.stream.write_all(b"550 Permission denied\r\n").await?;
                return Ok(());
            }

            let file_path = match safe_resolve_path(&self.cwd, &self.home_dir, filename) {
                Ok(p) => p,
                Err(e) => {
                    let error_msg = format!("550 Path resolution failed: {}\r\n", e);
                    self.stream.write_all(error_msg.as_bytes()).await?;
                    return Ok(());
                }
            };

            self.logger.lock().unwrap().debug(
                "FTP",
                &format!(
                    "STOR: input='{}', cwd='{}', home='{}', resolved='{}'",
                    filename, self.cwd, self.home_dir, file_path.display()
                ),
            );

            if !file_path.starts_with(&self.home_dir) {
                self.logger.lock().unwrap().warning(
                    "FTP",
                    &format!("STOR: Path outside home directory: {}", file_path.display()),
                );
                self.stream.write_all(b"550 Permission denied\r\n").await?;
                return Ok(());
            }

            if quota_mb > 0 {
                let current_usage = self.quota_cache.calculate_usage(&self.home_dir);
                let quota_bytes = quota_mb * 1024 * 1024;
                if current_usage >= quota_bytes {
                    self.logger.lock().unwrap().warning(
                        "FTP",
                        &format!("Quota exceeded for user {}: {}MB / {}MB", 
                            self.current_user.as_deref().unwrap_or("anonymous"),
                            current_usage / 1024 / 1024,
                            quota_mb),
                    );
                    self.stream.write_all(b"552 Quota exceeded, upload denied\r\n").await?;
                    return Ok(());
                }
            }

            let file_existed = file_path.exists();
            self.stream.write_all(b"150 Opening BINARY mode data connection\r\n").await?;

            let data_timeout = self.get_data_timeout();
            let data_result = get_data_connection(
                self.passive_mode,
                self.data_port,
                &self.data_addr,
                &self.remote_ip,
                &self.passive_listeners,
                data_timeout,
            ).await;

            let mut transfer_success = false;
            let mut total_written: u64 = 0;
            let global_speed_limit = self.config.lock().unwrap().ftp.max_speed_kbps;
            let effective_speed_limit = if user_speed_limit > 0 {
                Some(user_speed_limit)
            } else if global_speed_limit > 0 {
                Some(global_speed_limit)
            } else {
                None
            };
            let mut last_throttle_time = Instant::now();
            let mut bytes_since_throttle: u64 = 0;

            match data_result {
                Ok(mut data_stream) => {
                    let abort = Arc::clone(&self.abort_flag);
                    let file_result = if self.rest_offset > 0 {
                        tokio::fs::OpenOptions::new()
                            .write(true)
                            .create(true)
                            .truncate(false)
                            .open(&file_path)
                            .await
                    } else {
                        File::create(&file_path).await
                    };

                    match file_result {
                        Ok(mut file) => {
                            if self.rest_offset > 0 {
                                let _ = file.seek(std::io::SeekFrom::Start(self.rest_offset)).await;
                            }

                            let mut buf = [0u8; 8192];
                            transfer_success = true;
                            loop {
                                if abort.load(Ordering::Relaxed) {
                                    transfer_success = false;
                                    break;
                                }
                                match data_stream.read(&mut buf).await {
                                    Ok(0) => break,
                                    Ok(n) => {
                                        if file.write_all(&buf[..n]).await.is_err() {
                                            self.logger.lock().unwrap().error(
                                                "FTP",
                                                &format!("STOR write error for file: {}", file_path.display()),
                                            );
                                            transfer_success = false;
                                            break;
                                        }
                                        total_written += n as u64;
                                        
                                        if let Some(limit_kbps) = effective_speed_limit {
                                            bytes_since_throttle += n as u64;
                                            let elapsed = last_throttle_time.elapsed();
                                            if elapsed >= Duration::from_millis(100) {
                                                let max_bytes = (limit_kbps * 1024) as f64 * elapsed.as_secs_f64();
                                                if bytes_since_throttle > max_bytes as u64 {
                                                    let excess = bytes_since_throttle - max_bytes as u64;
                                                    let wait_secs = excess as f64 / (limit_kbps * 1024) as f64;
                                                    tokio::time::sleep(Duration::from_secs_f64(wait_secs)).await;
                                                }
                                                last_throttle_time = Instant::now();
                                                bytes_since_throttle = 0;
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        self.logger.lock().unwrap().error(
                                            "FTP",
                                            &format!("STOR read error from data stream: {}", e),
                                        );
                                        transfer_success = false;
                                        break;
                                    }
                                }
                            }
                            if transfer_success {
                                if let Err(e) = file.sync_all().await {
                                    self.logger.lock().unwrap().error(
                                        "FTP",
                                        &format!("Failed to sync file {:?}: {}", file_path, e),
                                    );
                                }
                                self.logger.lock().unwrap().info(
                                    "FTP",
                                    &format!("STOR completed: {} bytes written to {}", total_written, file_path.display()),
                                );
                            }
                        }
                        Err(e) => {
                            self.logger.lock().unwrap().error(
                                "FTP",
                                &format!("STOR failed to create file {}: {}", file_path.display(), e),
                            );
                        }
                    }
                    self.cleanup_data_connection().await;
                }
                Err(e) => {
                    self.logger.lock().unwrap().warning(
                        "FTP",
                        &format!("Failed to get data connection: {}", e),
                    );
                    self.cleanup_data_connection().await;
                    self.stream.write_all(b"425 Cannot open data connection\r\n").await?;
                    return Ok(());
                }
            }

            if transfer_success {
                self.stream.write_all(b"226 Transfer complete\r\n").await?;
                self.quota_cache.invalidate(&self.home_dir);

                let uploaded_size = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(total_written);
                if file_existed {
                    self.file_logger.lock().unwrap().log_update(
                        self.current_user.as_deref().unwrap_or("anonymous"),
                        &self.remote_ip,
                        &file_path.to_string_lossy(),
                        uploaded_size,
                        "FTP",
                    );
                } else {
                    self.file_logger.lock().unwrap().log_upload(
                        self.current_user.as_deref().unwrap_or("anonymous"),
                        &self.remote_ip,
                        &file_path.to_string_lossy(),
                        uploaded_size,
                        "FTP",
                    );
                }

                self.logger.lock().unwrap().client_action(
                    "FTP",
                    &format!("Uploaded: {} ({} bytes) at offset {}", filename, uploaded_size, self.rest_offset),
                    &self.remote_ip,
                    self.current_user.as_deref(),
                    "UPLOAD",
                );
            } else {
                self.stream.write_all(b"451 Transfer failed\r\n").await?;
                self.file_logger.lock().unwrap().log_failed(
                    self.current_user.as_deref().unwrap_or("anonymous"),
                    &self.remote_ip,
                    "UPLOAD",
                    &file_path.to_string_lossy(),
                    "FTP",
                    "Transfer failed",
                );
            }

            self.rest_offset = 0;
        }
        Ok(())
    }

    pub async fn cmd_appe(&mut self, arg: Option<&str>) -> Result<()> {
        if !self.authenticated {
            self.stream.write_all(b"530 Not logged in\r\n").await?;
            return Ok(());
        }

        if let Some(filename) = arg {
            let can_append = {
                let users = self.user_manager.lock().unwrap();
                let user = self.current_user.as_ref().and_then(|u| users.get_user(u));
                user.is_none_or(|u| u.permissions.can_append)
            };

            if !can_append {
                self.stream.write_all(b"550 Permission denied\r\n").await?;
                return Ok(());
            }

            let file_path = match safe_resolve_path(&self.cwd, &self.home_dir, filename) {
                Ok(p) => p,
                Err(e) => {
                    let error_msg = format!("550 Path resolution failed: {}\r\n", e);
                    self.stream.write_all(error_msg.as_bytes()).await?;
                    return Ok(());
                }
            };
            if !file_path.starts_with(&self.home_dir) {
                self.stream.write_all(b"550 Permission denied\r\n").await?;
                return Ok(());
            }
            self.stream.write_all(b"150 Opening BINARY mode data connection for append\r\n").await?;

            let data_timeout = self.get_data_timeout();
            let data_result = get_data_connection(
                self.passive_mode,
                self.data_port,
                &self.data_addr,
                &self.remote_ip,
                &self.passive_listeners,
                data_timeout,
            ).await;

            match data_result {
                Ok(mut data_stream) => {
                    let abort = Arc::clone(&self.abort_flag);
                    if let Ok(mut file) = tokio::fs::OpenOptions::new()
                        .append(true)
                        .create(true)
                        .open(&file_path)
                        .await
                    {
                        let mut buf = [0u8; 8192];
                        loop {
                            if abort.load(Ordering::Relaxed) {
                                break;
                            }
                            match data_stream.read(&mut buf).await {
                                Ok(0) => break,
                                Ok(n) => {
                                    if file.write_all(&buf[..n]).await.is_err() {
                                        break;
                                    }
                                }
                                Err(_) => break,
                            }
                        }
                    }
                    self.cleanup_data_connection().await;
                    self.stream.write_all(b"226 Transfer complete\r\n").await?;
                }
                Err(e) => {
                    self.logger.lock().unwrap().warning(
                        "FTP",
                        &format!("Failed to get data connection: {}", e),
                    );
                    self.cleanup_data_connection().await;
                    self.stream.write_all(b"425 Cannot open data connection\r\n").await?;
                    return Ok(());
                }
            }

            let appended_size = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);
            self.file_logger.lock().unwrap().log(FileLogInfo {
                username: self.current_user.as_deref().unwrap_or("anonymous"),
                client_ip: &self.remote_ip,
                operation: "APPEND",
                file_path: &file_path.to_string_lossy(),
                file_size: appended_size,
                protocol: "FTP",
                success: true,
                message: "文件追加成功",
            });

            self.logger.lock().unwrap().client_action(
                "FTP",
                &format!("Appended: {}", filename),
                &self.remote_ip,
                self.current_user.as_deref(),
                "APPEND",
            );
        }
        Ok(())
    }

    pub async fn cmd_stou(&mut self, _arg: Option<&str>) -> Result<()> {
        if !self.authenticated {
            self.stream.write_all(b"530 Not logged in\r\n").await?;
            return Ok(());
        }

        let can_write = {
            let users = self.user_manager.lock().unwrap();
            let user = self.current_user.as_ref().and_then(|u| users.get_user(u));
            user.is_none_or(|u| u.permissions.can_write)
        };

        if !can_write {
            self.stream.write_all(b"550 Permission denied\r\n").await?;
            return Ok(());
        }

        let cwd_path = std::path::Path::new(&self.cwd);
        let mut unique_name = format!("ftp_{}", chrono::Utc::now().format("%Y%m%d_%H%M%S_%f"));
        let mut file_path = cwd_path.join(&unique_name);
        let mut counter = 0u32;

        while file_path.exists() {
            counter += 1;
            unique_name = format!("ftp_{}_{}", chrono::Utc::now().format("%Y%m%d_%H%M%S"), counter);
            file_path = cwd_path.join(&unique_name);
        }

        let home_path = std::path::Path::new(&self.home_dir);
        if !file_path.starts_with(home_path) {
            self.stream.write_all(b"550 Permission denied\r\n").await?;
            return Ok(());
        }

        self.stream.write_all(format!("150 FILE: {}\r\n", unique_name).as_bytes()).await?;

        let data_timeout = self.get_data_timeout();
        let data_result = get_data_connection(
            self.passive_mode,
            self.data_port,
            &self.data_addr,
            &self.remote_ip,
            &self.passive_listeners,
            data_timeout,
        ).await;

        let mut transfer_success = false;
        let mut total_written: u64 = 0;

        match data_result {
            Ok(mut data_stream) => {
                let abort = Arc::clone(&self.abort_flag);
                match File::create(&file_path).await {
                    Ok(mut file) => {
                        let mut buf = [0u8; 8192];
                        transfer_success = true;
                        loop {
                            if abort.load(Ordering::Relaxed) {
                                transfer_success = false;
                                break;
                            }
                            match data_stream.read(&mut buf).await {
                                Ok(0) => break,
                                Ok(n) => {
                                    if file.write_all(&buf[..n]).await.is_err() {
                                        transfer_success = false;
                                        break;
                                    }
                                    total_written += n as u64;
                                }
                                Err(_) => {
                                    transfer_success = false;
                                    break;
                                }
                            }
                        }
                        if transfer_success
                            && let Err(e) = file.sync_all().await {
                                self.logger.lock().unwrap().error(
                                    "FTP",
                                    &format!("STOU sync error: {}", e),
                                );
                            }
                    }
                    Err(e) => {
                        self.logger.lock().unwrap().error(
                            "FTP",
                            &format!("STOU failed to create file: {}", e),
                        );
                    }
                }
                self.cleanup_data_connection().await;
            }
            Err(e) => {
                self.logger.lock().unwrap().warning(
                    "FTP",
                    &format!("Failed to get data connection: {}", e),
                );
                self.cleanup_data_connection().await;
                self.stream.write_all(b"425 Cannot open data connection\r\n").await?;
                return Ok(());
            }
        }

        if transfer_success {
            self.stream.write_all(format!("226 Transfer complete, file: {}\r\n", unique_name).as_bytes()).await?;

            let uploaded_size = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(total_written);
            self.file_logger.lock().unwrap().log_upload(
                self.current_user.as_deref().unwrap_or("anonymous"),
                &self.remote_ip,
                &file_path.to_string_lossy(),
                uploaded_size,
                "FTP",
            );

            self.logger.lock().unwrap().client_action(
                "FTP",
                &format!("STOU uploaded: {} ({} bytes)", unique_name, uploaded_size),
                &self.remote_ip,
                self.current_user.as_deref(),
                "UPLOAD",
            );
        } else {
            self.stream.write_all(b"451 Transfer failed\r\n").await?;
        }

        Ok(())
    }
}
