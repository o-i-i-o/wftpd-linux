use anyhow::Result;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use super::super::data_connection::get_data_connection;
use super::super::handler::FtpSession;
use super::super::utils::{build_mlst_facts, get_file_mtime, safe_resolve_path, escape_mlst_filename};
use crate::core::file_logger::FileLogInfo;

impl FtpSession {
    fn get_data_timeout(&self) -> u64 {
        self.config.lock().unwrap().ftp.data_timeout
    }

    pub fn cmd_list(&mut self, _arg: Option<&str>) -> Result<()> {
        if !self.authenticated {
            self.stream.write_all(b"530 Not logged in\r\n")?;
            return Ok(());
        }

        {
            let users = self.user_manager.lock().unwrap();
            let user = self.current_user.as_ref().and_then(|u| users.get_user(u));
            if let Some(user) = user {
                if !user.permissions.can_list {
                    self.stream.write_all(b"550 Permission denied\r\n")?;
                    return Ok(());
                }
            }
        }

        self.stream.write_all(b"150 Here comes the directory listing\r\n")?;

        let cwd = self.cwd.clone();
        let data_timeout = self.get_data_timeout();
        let data_result = get_data_connection(
            self.passive_mode,
            self.data_port,
            &self.data_addr,
            &self.remote_ip,
            &self.passive_listeners,
            data_timeout,
        );
        
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
                                let _ = data_stream.write_all(line.as_bytes());
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
                self.cleanup_data_connection();
                self.stream.write_all(b"226 Transfer complete\r\n")?;
            }
            Err(e) => {
                self.logger.lock().unwrap().warning(
                    "FTP",
                    &format!("Failed to get data connection: {}", e),
                );
                self.cleanup_data_connection();
                self.stream.write_all(b"425 Cannot open data connection\r\n")?;
            }
        }

        Ok(())
    }

    pub fn cmd_mlsd(&mut self) -> Result<()> {
        if !self.authenticated {
            self.stream.write_all(b"530 Not logged in\r\n")?;
            return Ok(());
        }

        {
            let users = self.user_manager.lock().unwrap();
            let user = self.current_user.as_ref().and_then(|u| users.get_user(u));
            if let Some(user) = user {
                if !user.permissions.can_list {
                    self.stream.write_all(b"550 Permission denied\r\n")?;
                    return Ok(());
                }
            }
        }

        self.stream.write_all(b"150 Here comes the directory listing\r\n")?;

        let data_timeout = self.get_data_timeout();
        let data_result = get_data_connection(
            self.passive_mode,
            self.data_port,
            &self.data_addr,
            &self.remote_ip,
            &self.passive_listeners,
            data_timeout,
        );

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
                            let _ = data_stream.write_all(line.as_bytes());
                        }
                    }
                }
                self.cleanup_data_connection();
                self.stream.write_all(b"226 Transfer complete\r\n")?;
            }
            Err(e) => {
                self.logger.lock().unwrap().warning(
                    "FTP",
                    &format!("Failed to get data connection: {}", e),
                );
                self.cleanup_data_connection();
                self.stream.write_all(b"425 Cannot open data connection\r\n")?;
            }
        }
        Ok(())
    }

    pub fn cmd_retr(&mut self, arg: Option<&str>) -> Result<()> {
        if !self.authenticated {
            self.stream.write_all(b"530 Not logged in\r\n")?;
            return Ok(());
        }

        if let Some(filename) = arg {
            let file_path = safe_resolve_path(&self.cwd, &self.home_dir, filename);
            
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
                self.stream.write_all(b"550 File not found\r\n")?;
                return Ok(());
            }

            {
                let users = self.user_manager.lock().unwrap();
                let user = self.current_user.as_ref().and_then(|u| users.get_user(u));

                if let Some(user) = user {
                    if !user.permissions.can_read {
                        self.stream.write_all(b"550 Permission denied\r\n")?;
                        return Ok(());
                    }
                }
            }

            let file_size = std::fs::metadata(&file_path)?.len();
            let remaining = if self.rest_offset > 0 && self.rest_offset < file_size {
                file_size - self.rest_offset
            } else {
                file_size
            };

            self.stream.write_all(
                format!("150 Opening BINARY mode data connection ({} bytes)\r\n", remaining)
                    .as_bytes(),
            )?;

            let data_timeout = self.get_data_timeout();
            let data_result = get_data_connection(
                self.passive_mode,
                self.data_port,
                &self.data_addr,
                &self.remote_ip,
                &self.passive_listeners,
                data_timeout,
            );

            match data_result {
                Ok(mut data_stream) => {
                    let abort = Arc::clone(&self.abort_flag);
                    if let Ok(mut file) = std::fs::File::open(&file_path) {
                        use std::io::Seek;
                        if self.rest_offset > 0 {
                            let _ = file.seek(std::io::SeekFrom::Start(self.rest_offset));
                        }

                        let mut buf = [0u8; 8192];
                        loop {
                            if abort.load(Ordering::Relaxed) {
                                break;
                            }
                            match file.read(&mut buf) {
                                Ok(0) => break,
                                Ok(n) => {
                                    if data_stream.write_all(&buf[..n]).is_err() {
                                        break;
                                    }
                                }
                                Err(_) => break,
                            }
                        }
                    }
                    self.cleanup_data_connection();
                    self.stream.write_all(b"226 Transfer complete\r\n")?;
                }
                Err(e) => {
                    self.logger.lock().unwrap().warning(
                        "FTP",
                        &format!("Failed to get data connection: {}", e),
                    );
                    self.cleanup_data_connection();
                    self.stream.write_all(b"425 Cannot open data connection\r\n")?;
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

    pub fn cmd_stor(&mut self, arg: Option<&str>) -> Result<()> {
        if !self.authenticated {
            self.stream.write_all(b"530 Not logged in\r\n")?;
            return Ok(());
        }

        if let Some(filename) = arg {
            {
                let users = self.user_manager.lock().unwrap();
                let user = self.current_user.as_ref().and_then(|u| users.get_user(u));

                if let Some(user) = user {
                    if !user.permissions.can_write {
                        self.stream.write_all(b"550 Permission denied\r\n")?;
                        return Ok(());
                    }
                }
            }

            let file_path = safe_resolve_path(&self.cwd, &self.home_dir, filename);
            
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
                self.stream.write_all(b"550 Permission denied\r\n")?;
                return Ok(());
            }
            let file_existed = file_path.exists();
            self.stream.write_all(b"150 Opening BINARY mode data connection\r\n")?;

            let data_timeout = self.get_data_timeout();
            let data_result = get_data_connection(
                self.passive_mode,
                self.data_port,
                &self.data_addr,
                &self.remote_ip,
                &self.passive_listeners,
                data_timeout,
            );

            match data_result {
                Ok(mut data_stream) => {
                    let abort = Arc::clone(&self.abort_flag);
                    let file_result = if self.rest_offset > 0 {
                        std::fs::OpenOptions::new()
                            .write(true)
                            .create(true)
                            .truncate(false)
                            .open(&file_path)
                    } else {
                        std::fs::File::create(&file_path)
                    };

                    if let Ok(mut file) = file_result {
                        use std::io::Seek;
                        if self.rest_offset > 0 {
                            let _ = file.seek(std::io::SeekFrom::Start(self.rest_offset));
                        }

                        let mut buf = [0u8; 8192];
                        loop {
                            if abort.load(Ordering::Relaxed) {
                                break;
                            }
                            match data_stream.read(&mut buf) {
                                Ok(0) => break,
                                Ok(n) => {
                                    if file.write_all(&buf[..n]).is_err() {
                                        break;
                                    }
                                }
                                Err(_) => break,
                            }
                        }
                    }
                    self.cleanup_data_connection();
                    self.stream.write_all(b"226 Transfer complete\r\n")?;
                }
                Err(e) => {
                    self.logger.lock().unwrap().warning(
                        "FTP",
                        &format!("Failed to get data connection: {}", e),
                    );
                    self.cleanup_data_connection();
                    self.stream.write_all(b"425 Cannot open data connection\r\n")?;
                    return Ok(());
                }
            }

            let uploaded_size = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);
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
                &format!("Uploaded: {} at offset {}", filename, self.rest_offset),
                &self.remote_ip,
                self.current_user.as_deref(),
                "UPLOAD",
            );

            self.rest_offset = 0;
        }
        Ok(())
    }

    pub fn cmd_appe(&mut self, arg: Option<&str>) -> Result<()> {
        if !self.authenticated {
            self.stream.write_all(b"530 Not logged in\r\n")?;
            return Ok(());
        }

        if let Some(filename) = arg {
            {
                let users = self.user_manager.lock().unwrap();
                let user = self.current_user.as_ref().and_then(|u| users.get_user(u));

                if let Some(user) = user {
                    if !user.permissions.can_append {
                        self.stream.write_all(b"550 Permission denied\r\n")?;
                        return Ok(());
                    }
                }
            }

            let file_path = safe_resolve_path(&self.cwd, &self.home_dir, filename);
            if !file_path.starts_with(&self.home_dir) {
                self.stream.write_all(b"550 Permission denied\r\n")?;
                return Ok(());
            }
            self.stream.write_all(b"150 Opening BINARY mode data connection for append\r\n")?;

            let data_timeout = self.get_data_timeout();
            let data_result = get_data_connection(
                self.passive_mode,
                self.data_port,
                &self.data_addr,
                &self.remote_ip,
                &self.passive_listeners,
                data_timeout,
            );

            match data_result {
                Ok(mut data_stream) => {
                    let abort = Arc::clone(&self.abort_flag);
                    if let Ok(mut file) = std::fs::OpenOptions::new()
                        .append(true)
                        .create(true)
                        .open(&file_path)
                    {
                        let mut buf = [0u8; 8192];
                        loop {
                            if abort.load(Ordering::Relaxed) {
                                break;
                            }
                            match data_stream.read(&mut buf) {
                                Ok(0) => break,
                                Ok(n) => {
                                    if file.write_all(&buf[..n]).is_err() {
                                        break;
                                    }
                                }
                                Err(_) => break,
                            }
                        }
                    }
                    self.cleanup_data_connection();
                    self.stream.write_all(b"226 Transfer complete\r\n")?;
                }
                Err(e) => {
                    self.logger.lock().unwrap().warning(
                        "FTP",
                        &format!("Failed to get data connection: {}", e),
                    );
                    self.cleanup_data_connection();
                    self.stream.write_all(b"425 Cannot open data connection\r\n")?;
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
}
