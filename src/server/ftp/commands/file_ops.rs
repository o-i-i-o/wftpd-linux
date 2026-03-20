use anyhow::Result;
use std::path::Path;

use super::super::handler::FtpSession;
use super::super::utils::{get_file_mtime_raw, safe_resolve_path};

impl FtpSession {
    pub async fn cmd_dele(&mut self, arg: Option<&str>) -> Result<()> {
        if !self.authenticated {
            self.stream.write_all(b"530 Not logged in\r\n").await?;
            return Ok(());
        }

        let can_delete = {
            let users = self.user_manager.lock().unwrap();
            let user = self.current_user.as_ref().and_then(|u| users.get_user(u));
            user.is_none_or(|u| u.permissions.can_delete)
        };

        if !can_delete {
            self.stream.write_all(b"550 Permission denied\r\n").await?;
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
            let home_path = Path::new(&self.home_dir);
            if !file_path.starts_with(home_path) {
                self.stream.write_all(b"550 Permission denied\r\n").await?;
                return Ok(());
            }
            match std::fs::remove_file(&file_path) {
                Ok(_) => {
                    self.stream.write_all(b"250 File deleted\r\n").await?;
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
                    self.stream.write_all(error_msg.as_bytes()).await?;
                }
            }
        }
        Ok(())
    }

    pub async fn cmd_rnfr(&mut self, arg: Option<&str>) -> Result<()> {
        if !self.authenticated {
            self.stream.write_all(b"530 Not logged in\r\n").await?;
            return Ok(());
        }

        let can_rename = {
            let users = self.user_manager.lock().unwrap();
            let user = self.current_user.as_ref().and_then(|u| users.get_user(u));
            user.is_none_or(|u| u.permissions.can_rename)
        };

        if !can_rename {
            self.stream.write_all(b"550 Permission denied\r\n").await?;
            return Ok(());
        }

        if let Some(from_name) = arg {
            self.logger.lock().unwrap().debug(
                "FTP",
                &format!("RNFR: input='{}', cwd='{}', home='{}'", from_name, self.cwd, self.home_dir),
            );
            
            let from_path = match safe_resolve_path(&self.cwd, &self.home_dir, from_name) {
                Ok(p) => p,
                Err(e) => {
                    let error_msg = format!("550 Path resolution failed: {}\r\n", e);
                    self.stream.write_all(error_msg.as_bytes()).await?;
                    self.logger.lock().unwrap().warning(
                        "FTP",
                        &format!("RNFR path resolution failed: {}", e),
                    );
                    return Ok(());
                }
            };
            
            self.logger.lock().unwrap().debug(
                "FTP",
                &format!("RNFR: resolved path='{}', exists={}", from_path.display(), from_path.exists()),
            );
            
            let home_path = Path::new(&self.home_dir);
            let home_canon = match home_path.canonicalize() {
                Ok(c) => c,
                Err(_) => home_path.to_path_buf(),
            };
            
            if from_path.exists() && from_path.starts_with(&home_canon) {
                self.rename_from = Some(from_path.to_string_lossy().to_string());
                self.stream.write_all(b"350 File exists, ready for destination name\r\n").await?;
                self.logger.lock().unwrap().debug(
                    "FTP",
                    &format!("RNFR: stored path for rename: {}", from_path.display()),
                );
            } else {
                let reason = if !from_path.exists() {
                    "file does not exist"
                } else {
                    "path outside home directory"
                };
                self.logger.lock().unwrap().warning(
                    "FTP",
                    &format!("RNFR failed for '{}': {}", from_path.display(), reason),
                );
                self.stream.write_all(b"550 File not found\r\n").await?;
            }
        } else {
            self.stream.write_all(b"501 Syntax error: RNFR requires parameter\r\n").await?;
        }
        Ok(())
    }

    pub async fn cmd_rnto(&mut self, arg: Option<&str>) -> Result<()> {
        if !self.authenticated {
            self.stream.write_all(b"530 Not logged in\r\n").await?;
            self.rename_from = None;
            return Ok(());
        }

        if let Some(ref from_path) = self.rename_from {
            if let Some(to_name) = arg {
                self.logger.lock().unwrap().debug(
                    "FTP",
                    &format!("RNTO: input='{}', from_path='{}', cwd='{}', home='{}'", 
                        to_name, from_path, self.cwd, self.home_dir),
                );
                
                let to_path = match safe_resolve_path(&self.cwd, &self.home_dir, to_name) {
                    Ok(p) => p,
                    Err(e) => {
                        let error_msg = format!("550 Path resolution failed: {}\r\n", e);
                        self.stream.write_all(error_msg.as_bytes()).await?;
                        self.logger.lock().unwrap().warning(
                            "FTP",
                            &format!("RNTO path resolution failed: {}", e),
                        );
                        self.rename_from = None;
                        return Ok(());
                    }
                };
                
                self.logger.lock().unwrap().debug(
                    "FTP",
                    &format!("RNTO: resolved to_path='{}'", to_path.display()),
                );
                
                let home_path = Path::new(&self.home_dir);
                let home_canon = match home_path.canonicalize() {
                    Ok(c) => c,
                    Err(_) => home_path.to_path_buf(),
                };
                
                if !to_path.starts_with(&home_canon) {
                    self.stream.write_all(b"550 Permission denied\r\n").await?;
                    self.logger.lock().unwrap().warning(
                        "FTP",
                        &format!("RNTO: destination path '{}' outside home directory", to_path.display()),
                    );
                    self.rename_from = None;
                    return Ok(());
                }
                
                let from_path_buf = Path::new(from_path);
                let cwd_path = Path::new(&self.cwd);
                if from_path_buf == cwd_path {
                    self.stream.write_all(b"550 Cannot rename current working directory\r\n").await?;
                    self.rename_from = None;
                    return Ok(());
                }
                
                if cwd_path.starts_with(from_path_buf) && cwd_path != from_path_buf {
                    self.stream.write_all(b"550 Cannot rename parent of current working directory\r\n").await?;
                    self.rename_from = None;
                    return Ok(());
                }
                
                match std::fs::rename(from_path, &to_path) {
                    Ok(_) => {
                        self.stream.write_all(b"250 Rename successful\r\n").await?;
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
                    }
                    Err(e) => {
                        let error_msg = format!("550 Rename failed: {}\r\n", e);
                        self.stream.write_all(error_msg.as_bytes()).await?;
                        self.logger.lock().unwrap().error(
                            "FTP",
                            &format!("RNTO: rename failed from '{}' to '{}': {}", from_path, to_path.display(), e),
                        );
                    }
                }
            } else {
                self.stream.write_all(b"501 Syntax error: RNTO requires parameter\r\n").await?;
            }
        } else {
            self.stream.write_all(b"503 Bad sequence of commands\r\n").await?;
        }
        self.rename_from = None;
        Ok(())
    }

    pub async fn cmd_size(&mut self, arg: Option<&str>) -> Result<()> {
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
            let home_path = Path::new(&self.home_dir);
            if !file_path.starts_with(home_path) {
                self.stream.write_all(b"550 Permission denied\r\n").await?;
                return Ok(());
            }
            if let Ok(metadata) = std::fs::metadata(&file_path) {
                self.stream.write_all(format!("213 {}\r\n", metadata.len()).as_bytes()).await?;
            } else {
                self.stream.write_all(b"550 File not found\r\n").await?;
            }
        } else {
            self.stream.write_all(b"501 Syntax error: SIZE requires parameter\r\n").await?;
        }
        Ok(())
    }

    pub async fn cmd_mdtm(&mut self, arg: Option<&str>) -> Result<()> {
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
            let home_path = Path::new(&self.home_dir);
            if !file_path.starts_with(home_path) {
                self.stream.write_all(b"550 Permission denied\r\n").await?;
                return Ok(());
            }
            if let Ok(metadata) = std::fs::metadata(&file_path) {
                let mtime = get_file_mtime_raw(&metadata);
                self.stream.write_all(format!("213 {}\r\n", mtime).as_bytes()).await?;
            } else {
                self.stream.write_all(b"550 File not found\r\n").await?;
            }
        } else {
            self.stream.write_all(b"501 Syntax error: MDTM requires parameter\r\n").await?;
        }
        Ok(())
    }
}
