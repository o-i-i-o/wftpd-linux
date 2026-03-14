use anyhow::Result;
use std::io::Write;

use super::super::handler::FtpSession;
use super::super::utils::{get_file_mtime_raw, safe_resolve_path};

impl FtpSession {
    pub fn cmd_dele(&mut self, arg: Option<&str>) -> Result<()> {
        if !self.authenticated {
            self.stream.write_all(b"530 Not logged in\r\n")?;
            return Ok(());
        }

        {
            let users = self.user_manager.lock().unwrap();
            let user = self.current_user.as_ref().and_then(|u| users.get_user(u));

            if let Some(user) = user {
                if !user.permissions.can_delete {
                    self.stream.write_all(b"550 Permission denied\r\n")?;
                    return Ok(());
                }
            }
        }

        if let Some(filename) = arg {
            let file_path = safe_resolve_path(&self.cwd, &self.home_dir, filename);
            if !file_path.starts_with(&self.home_dir) {
                self.stream.write_all(b"550 Permission denied\r\n")?;
                return Ok(());
            }
            match std::fs::remove_file(&file_path) {
                Ok(_) => {
                    self.stream.write_all(b"250 File deleted\r\n")?;
                    self.file_logger.lock().unwrap().log_delete(
                        self.current_user.as_deref().unwrap_or("anonymous"),
                        &self.remote_ip,
                        &file_path.to_string_lossy(),
                        "FTP",
                    );
                    self.logger.lock().unwrap().client_action(
                        "FTP",
                        &format!("Deleted: {}", filename),
                        &self.remote_ip,
                        self.current_user.as_deref(),
                        "DELETE",
                    );
                }
                Err(e) => {
                    let error_msg = format!("550 Delete operation failed: {}\r\n", e);
                    self.stream.write_all(error_msg.as_bytes())?;
                }
            }
        }
        Ok(())
    }

    pub fn cmd_rnfr(&mut self, arg: Option<&str>) -> Result<()> {
        if !self.authenticated {
            self.stream.write_all(b"530 Not logged in\r\n")?;
            return Ok(());
        }

        {
            let users = self.user_manager.lock().unwrap();
            let user = self.current_user.as_ref().and_then(|u| users.get_user(u));

            if let Some(user) = user {
                if !user.permissions.can_rename {
                    self.stream.write_all(b"550 Permission denied\r\n")?;
                    return Ok(());
                }
            }
        }

        if let Some(from_name) = arg {
            let from_path = safe_resolve_path(&self.cwd, &self.home_dir, from_name);
            if from_path.exists() && from_path.starts_with(&self.home_dir) {
                self.rename_from = Some(from_path.to_string_lossy().to_string());
                self.stream.write_all(b"350 File exists, ready for destination name\r\n")?;
            } else {
                self.stream.write_all(b"550 File not found\r\n")?;
            }
        }
        Ok(())
    }

    pub fn cmd_rnto(&mut self, arg: Option<&str>) -> Result<()> {
        if !self.authenticated {
            self.stream.write_all(b"530 Not logged in\r\n")?;
            self.rename_from = None;
            return Ok(());
        }

        if let Some(ref from_path) = self.rename_from {
            if let Some(to_name) = arg {
                let to_path = safe_resolve_path(&self.cwd, &self.home_dir, to_name);
                if !to_path.starts_with(&self.home_dir) {
                    self.stream.write_all(b"550 Permission denied\r\n")?;
                    self.rename_from = None;
                    return Ok(());
                }
                if std::fs::rename(from_path, &to_path).is_ok() {
                    self.stream.write_all(b"250 Rename successful\r\n")?;
                    self.file_logger.lock().unwrap().log_rename(
                        self.current_user.as_deref().unwrap_or("anonymous"),
                        &self.remote_ip,
                        from_path,
                        &to_path.to_string_lossy(),
                        "FTP",
                    );
                    self.logger.lock().unwrap().client_action(
                        "FTP",
                        &format!("Renamed: {} -> {}", from_path, to_path.display()),
                        &self.remote_ip,
                        self.current_user.as_deref(),
                        "RENAME",
                    );
                } else {
                    self.stream.write_all(b"550 Rename failed\r\n")?;
                }
            }
        } else {
            self.stream.write_all(b"503 Bad sequence of commands\r\n")?;
        }
        self.rename_from = None;
        Ok(())
    }

    pub fn cmd_size(&mut self, arg: Option<&str>) -> Result<()> {
        if let Some(filename) = arg {
            let file_path = safe_resolve_path(&self.cwd, &self.home_dir, filename);
            if file_path.starts_with(&self.home_dir) {
                if let Ok(metadata) = std::fs::metadata(&file_path) {
                    self.stream.write_all(format!("213 {}\r\n", metadata.len()).as_bytes())?;
                } else {
                    self.stream.write_all(b"550 File not found\r\n")?;
                }
            } else {
                self.stream.write_all(b"550 Permission denied\r\n")?;
            }
        }
        Ok(())
    }

    pub fn cmd_mdtm(&mut self, arg: Option<&str>) -> Result<()> {
        if let Some(filename) = arg {
            let file_path = safe_resolve_path(&self.cwd, &self.home_dir, filename);
            if file_path.starts_with(&self.home_dir) {
                if let Ok(metadata) = std::fs::metadata(&file_path) {
                    let mtime = get_file_mtime_raw(&metadata);
                    self.stream.write_all(format!("213 {}\r\n", mtime).as_bytes())?;
                } else {
                    self.stream.write_all(b"550 File not found\r\n")?;
                }
            } else {
                self.stream.write_all(b"550 Permission denied\r\n")?;
            }
        }
        Ok(())
    }
}
