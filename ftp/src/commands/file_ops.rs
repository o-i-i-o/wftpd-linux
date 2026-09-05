use anyhow::Result;
use std::path::Path;
use tracing::{debug, error, info, warn};

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
                    // 使用 tracing 记录日志
                    info!(
                        username = self.current_user.as_deref().unwrap_or("anonymous"),
                        client_ip = %self.remote_ip,
                        file = %filename,
                        "FTP 文件删除成功"
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
            // 使用 tracing 记录调试日志
            debug!(
                input = %from_name,
                cwd = %self.cwd,
                home = %self.home_dir,
                "FTP RNFR 命令参数"
            );

            let from_path = match safe_resolve_path(&self.cwd, &self.home_dir, from_name) {
                Ok(p) => p,
                Err(e) => {
                    let error_msg = format!("550 Path resolution failed: {}\r\n", e);
                    self.stream.write_all(error_msg.as_bytes()).await?;
                    warn!(error = %e, "FTP RNFR 路径解析失败");
                    return Ok(());
                }
            };

            // 使用 tracing 记录调试日志
            debug!(
                resolved = %from_path.display(),
                exists = from_path.exists(),
                "FTP RNFR 解析路径"
            );

            let home_path = Path::new(&self.home_dir);
            let home_canon = match home_path.canonicalize() {
                Ok(c) => c,
                Err(_) => home_path.to_path_buf(),
            };

            if from_path.exists() && from_path.starts_with(&home_canon) {
                self.rename_from = Some(from_path.to_string_lossy().to_string());
                self.stream
                    .write_all(b"350 File exists, ready for destination name\r\n")
                    .await?;
                // 使用 tracing 记录调试日志
                debug!(
                    from_path = %from_path.display(),
                    "FTP RNFR 存储路径"
                );
            } else {
                let reason = if !from_path.exists() {
                    "file does not exist"
                } else {
                    "path outside home directory"
                };
                // 使用 tracing 记录警告日志
                warn!(
                    from_path = %from_path.display(),
                    reason = reason,
                    "FTP RNFR 失败"
                );
                self.stream.write_all(b"550 File not found\r\n").await?;
            }
        } else {
            self.stream
                .write_all(b"501 Syntax error: RNFR requires parameter\r\n")
                .await?;
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
                // 使用 tracing 记录调试日志
                debug!(
                    input = %to_name,
                    from_path = %from_path,
                    cwd = %self.cwd,
                    home = %self.home_dir,
                    "FTP RNTO 命令参数"
                );

                let to_path = match safe_resolve_path(&self.cwd, &self.home_dir, to_name) {
                    Ok(p) => p,
                    Err(e) => {
                        let error_msg = format!("550 Path resolution failed: {}\r\n", e);
                        self.stream.write_all(error_msg.as_bytes()).await?;
                        // 使用 tracing 记录警告日志
                        warn!(error = %e, "FTP RNTO 路径解析失败");
                        self.rename_from = None;
                        return Ok(());
                    }
                };

                // 使用 tracing 记录调试日志
                debug!(
                    resolved_to_path = %to_path.display(),
                    "FTP RNTO 解析路径"
                );

                let home_path = Path::new(&self.home_dir);
                let home_canon = match home_path.canonicalize() {
                    Ok(c) => c,
                    Err(_) => home_path.to_path_buf(),
                };

                if !to_path.starts_with(&home_canon) {
                    self.stream.write_all(b"550 Permission denied\r\n").await?;
                    // 使用 tracing 记录警告日志
                    warn!(
                        to_path = %to_path.display(),
                        "FTP RNTO 目标路径超出主目录范围"
                    );
                    self.rename_from = None;
                    return Ok(());
                }

                let from_path_buf = Path::new(from_path);
                let cwd_path = Path::new(&self.cwd);
                if from_path_buf == cwd_path {
                    self.stream
                        .write_all(b"550 Cannot rename current working directory\r\n")
                        .await?;
                    self.rename_from = None;
                    return Ok(());
                }

                if cwd_path.starts_with(from_path_buf) && cwd_path != from_path_buf {
                    self.stream
                        .write_all(b"550 Cannot rename parent of current working directory\r\n")
                        .await?;
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
                        // 使用 tracing 记录客户端操作审计日志
                        info!(
                            username = self.current_user.as_deref().unwrap_or("anonymous"),
                            client_ip = %self.remote_ip,
                            from_path = %from_path,
                            to_path = %to_path.display(),
                            "FTP 文件重命名成功"
                        );
                    }
                    Err(e) => {
                        let error_msg = format!("550 Rename failed: {}\r\n", e);
                        self.stream.write_all(error_msg.as_bytes()).await?;
                        // 使用 tracing 记录错误日志
                        error!(
                            from_path = %from_path,
                            to_path = %to_path.display(),
                            error = %e,
                            "FTP RNTO 重命名失败"
                        );
                    }
                }
            } else {
                self.stream
                    .write_all(b"501 Syntax error: RNTO requires parameter\r\n")
                    .await?;
            }
        } else {
            self.stream
                .write_all(b"503 Bad sequence of commands\r\n")
                .await?;
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
                self.stream
                    .write_all(format!("213 {}\r\n", metadata.len()).as_bytes())
                    .await?;
            } else {
                self.stream.write_all(b"550 File not found\r\n").await?;
            }
        } else {
            self.stream
                .write_all(b"501 Syntax error: SIZE requires parameter\r\n")
                .await?;
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
                self.stream
                    .write_all(format!("213 {}\r\n", mtime).as_bytes())
                    .await?;
            } else {
                self.stream.write_all(b"550 File not found\r\n").await?;
            }
        } else {
            self.stream
                .write_all(b"501 Syntax error: MDTM requires parameter\r\n")
                .await?;
        }
        Ok(())
    }
}
