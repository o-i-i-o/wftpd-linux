use anyhow::Result;
use std::path::Path;
use tracing::info;

use super::super::handler::FtpSession;
use super::super::utils::{build_mlst_facts, safe_resolve_path, escape_mlst_filename, real_to_virtual_path};

impl FtpSession {
    pub async fn cmd_cwd(&mut self, arg: Option<&str>) -> Result<()> {
        if !self.authenticated {
            self.stream.write_all(b"530 Not logged in\r\n").await?;
            return Ok(());
        }

        if let Some(dir) = arg {
            let new_path = match safe_resolve_path(&self.cwd, &self.home_dir, dir) {
                Ok(p) => p,
                Err(e) => {
                    let error_msg = format!("550 Path resolution failed: {}\r\n", e);
                    self.stream.write_all(error_msg.as_bytes()).await?;
                    return Ok(());
                }
            };
            let home_path = Path::new(&self.home_dir);

            if new_path.exists() && new_path.is_dir() && new_path.starts_with(home_path) {
                self.cwd = new_path.to_string_lossy().to_string();
                let virtual_path = real_to_virtual_path(&self.cwd, &self.home_dir);
                self.stream.write_all(format!("250 \"{}\" is current directory\r\n", virtual_path).as_bytes()).await?;
            } else {
                self.stream.write_all(b"550 Failed to change directory: Permission denied or directory not found\r\n").await?;
            }
        }
        Ok(())
    }

    pub async fn cmd_cdup(&mut self) -> Result<()> {
        let new_path = match safe_resolve_path(&self.cwd, &self.home_dir, "..") {
            Ok(p) => p,
            Err(e) => {
                let error_msg = format!("550 Path resolution failed: {}\r\n", e);
                self.stream.write_all(error_msg.as_bytes()).await?;
                return Ok(());
            }
        };
        let home_path = Path::new(&self.home_dir);
        if new_path.starts_with(home_path) && new_path.exists() {
            self.cwd = new_path.to_string_lossy().to_string();
            self.stream.write_all(b"250 Directory changed\r\n").await?;
        } else {
            self.stream.write_all(b"550 Cannot change to parent directory: Permission denied\r\n").await?;
        }
        Ok(())
    }

    pub async fn cmd_mlst(&mut self, arg: Option<&str>) -> Result<()> {
        if !self.authenticated {
            self.stream.write_all(b"530 Not logged in\r\n").await?;
            return Ok(());
        }

        let target_path = if let Some(path_arg) = arg {
            match safe_resolve_path(&self.cwd, &self.home_dir, path_arg) {
                Ok(p) => p,
                Err(e) => {
                    let error_msg = format!("550 Path resolution failed: {}\r\n", e);
                    self.stream.write_all(error_msg.as_bytes()).await?;
                    return Ok(());
                }
            }
        } else {
            Path::new(&self.cwd).to_path_buf()
        };

        let home_path = Path::new(&self.home_dir);
        if target_path.exists() && target_path.starts_with(home_path) {
            if let Ok(metadata) = target_path.metadata() {
                let facts = build_mlst_facts(&metadata);
                let virtual_path = real_to_virtual_path(&target_path.to_string_lossy(), &self.home_dir);
                let escaped_name = escape_mlst_filename(&virtual_path);
                self.stream.write_all(format!("250-Listing {}\r\n {}{}\r\n250 End\r\n", 
                    virtual_path, facts, escaped_name).as_bytes()).await?;
            } else {
                self.stream.write_all(b"550 Failed to get file info\r\n").await?;
            }
        } else {
            self.stream.write_all(b"550 File not found\r\n").await?;
        }
        Ok(())
    }

    pub async fn cmd_mkd(&mut self, arg: Option<&str>) -> Result<()> {
        if !self.authenticated {
            self.stream.write_all(b"530 Not logged in\r\n").await?;
            return Ok(());
        }

        let can_mkdir = {
            let users = self.user_manager.lock().unwrap();
            let user = self.current_user.as_ref().and_then(|u| users.get_user(u));
            user.is_none_or(|u| u.permissions.can_mkdir)
        };

        if !can_mkdir {
            self.stream.write_all(b"550 Permission denied\r\n").await?;
            return Ok(());
        }

        if let Some(dirname) = arg {
            let dir_path = match safe_resolve_path(&self.cwd, &self.home_dir, dirname) {
                Ok(p) => p,
                Err(e) => {
                    let error_msg = format!("550 Path resolution failed: {}\r\n", e);
                    self.stream.write_all(error_msg.as_bytes()).await?;
                    return Ok(());
                }
            };
            let home_path = Path::new(&self.home_dir);
            if !dir_path.starts_with(home_path) {
                self.stream.write_all(b"550 Permission denied\r\n").await?;
                return Ok(());
            }
            if std::fs::create_dir_all(&dir_path).is_ok() {
                let virtual_path = real_to_virtual_path(&dir_path.to_string_lossy(), &self.home_dir);
                self.stream.write_all(format!("257 \"{}\" created\r\n", virtual_path).as_bytes()).await?;
                self.file_logger.lock().unwrap().log_mkdir(
                    self.current_user.as_deref().unwrap_or("anonymous"),
                    &self.remote_ip,
                    &dir_path.to_string_lossy(),
                    "FTP",
                );
                // 使用 tracing 记录客户端操作审计日志
                info!(
                    username = self.current_user.as_deref().unwrap_or("anonymous"),
                    client_ip = %self.remote_ip,
                    path = %dir_path.to_string_lossy(),
                    "FTP 创建目录"
                );
            } else {
                self.stream.write_all(b"550 Create directory operation failed\r\n").await?;
            }
        }
        Ok(())
    }

    pub async fn cmd_rmd(&mut self, arg: Option<&str>) -> Result<()> {
        if !self.authenticated {
            self.stream.write_all(b"530 Not logged in\r\n").await?;
            return Ok(());
        }

        let can_rmdir = {
            let users = self.user_manager.lock().unwrap();
            let user = self.current_user.as_ref().and_then(|u| users.get_user(u));
            user.is_none_or(|u| u.permissions.can_rmdir)
        };

        if !can_rmdir {
            self.stream.write_all(b"550 Permission denied\r\n").await?;
            return Ok(());
        }

        if let Some(dirname) = arg {
            let dir_path = match safe_resolve_path(&self.cwd, &self.home_dir, dirname) {
                Ok(p) => p,
                Err(e) => {
                    let error_msg = format!("550 Path resolution failed: {}\r\n", e);
                    self.stream.write_all(error_msg.as_bytes()).await?;
                    return Ok(());
                }
            };
            let home_path = Path::new(&self.home_dir);
            if !dir_path.starts_with(home_path) {
                self.stream.write_all(b"550 Permission denied\r\n").await?;
                return Ok(());
            }
            
            let cwd_path = Path::new(&self.cwd);
            if dir_path == cwd_path {
                self.stream.write_all(b"550 Cannot remove current working directory\r\n").await?;
                return Ok(());
            }
            
            if cwd_path.starts_with(&dir_path) && cwd_path != dir_path {
                self.stream.write_all(b"550 Cannot remove parent of current working directory\r\n").await?;
                return Ok(());
            }
            
            if std::fs::remove_dir_all(&dir_path).is_ok() {
                self.stream.write_all(b"250 Directory removed\r\n").await?;
                self.file_logger.lock().unwrap().log_rmdir(
                    self.current_user.as_deref().unwrap_or("anonymous"),
                    &self.remote_ip,
                    &dir_path.to_string_lossy(),
                    "FTP",
                );
                // 使用 tracing 记录客户端操作审计日志
                info!(
                    username = self.current_user.as_deref().unwrap_or("anonymous"),
                    client_ip = %self.remote_ip,
                    path = %dir_path.to_string_lossy(),
                    "FTP 删除目录"
                );
            } else {
                self.stream.write_all(b"550 Remove directory operation failed\r\n").await?;
            }
        }
        Ok(())
    }
}
