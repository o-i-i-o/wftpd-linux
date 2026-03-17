use anyhow::Result;
use std::io::Write;
use std::path::Path;

use super::super::handler::FtpSession;
use super::super::utils::{build_mlst_facts, safe_resolve_path, escape_mlst_filename};

impl FtpSession {
    pub fn cmd_cwd(&mut self, arg: Option<&str>) -> Result<()> {
        if !self.authenticated {
            self.stream.write_all(b"530 Not logged in\r\n")?;
            return Ok(());
        }

        if let Some(dir) = arg {
            let new_path = safe_resolve_path(&self.cwd, &self.home_dir, dir);

            if new_path.exists() && new_path.is_dir() && new_path.starts_with(&self.home_dir) {
                self.cwd = new_path.to_string_lossy().to_string();
                self.stream.write_all(format!("250 \"{}\" is current directory\r\n", self.cwd).as_bytes())?;
            } else {
                self.stream.write_all(b"550 Failed to change directory: Permission denied or directory not found\r\n")?;
            }
        }
        Ok(())
    }

    pub fn cmd_cdup(&mut self) -> Result<()> {
        let new_path = safe_resolve_path(&self.cwd, &self.home_dir, "..");
        if new_path.starts_with(&self.home_dir) && new_path.exists() {
            self.cwd = new_path.to_string_lossy().to_string();
            self.stream.write_all(b"250 Directory changed\r\n")?;
        } else {
            self.stream.write_all(b"550 Cannot change to parent directory: Permission denied\r\n")?;
        }
        Ok(())
    }

    pub fn cmd_mlst(&mut self, arg: Option<&str>) -> Result<()> {
        if !self.authenticated {
            self.stream.write_all(b"530 Not logged in\r\n")?;
            return Ok(());
        }

        let target_path = if let Some(path_arg) = arg {
            safe_resolve_path(&self.cwd, &self.home_dir, path_arg)
        } else {
            Path::new(&self.cwd).to_path_buf()
        };

        if target_path.exists() && target_path.starts_with(&self.home_dir) {
            if let Ok(metadata) = target_path.metadata() {
                let facts = build_mlst_facts(&metadata);
                let name = target_path.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| target_path.to_string_lossy().to_string());
                let escaped_name = escape_mlst_filename(&name);
                self.stream.write_all(format!("250-Listing {}\r\n {}{}\r\n250 End\r\n", 
                    target_path.display(), facts, escaped_name).as_bytes())?;
            } else {
                self.stream.write_all(b"550 Failed to get file info\r\n")?;
            }
        } else {
            self.stream.write_all(b"550 File not found\r\n")?;
        }
        Ok(())
    }

    pub fn cmd_mkd(&mut self, arg: Option<&str>) -> Result<()> {
        if !self.authenticated {
            self.stream.write_all(b"530 Not logged in\r\n")?;
            return Ok(());
        }

        {
            let users = self.user_manager.lock().unwrap();
            let user = self.current_user.as_ref().and_then(|u| users.get_user(u));

            if let Some(user) = user {
                if !user.permissions.can_mkdir {
                    self.stream.write_all(b"550 Permission denied\r\n")?;
                    return Ok(());
                }
            }
        }

        if let Some(dirname) = arg {
            let dir_path = safe_resolve_path(&self.cwd, &self.home_dir, dirname);
            if !dir_path.starts_with(&self.home_dir) {
                self.stream.write_all(b"550 Permission denied\r\n")?;
                return Ok(());
            }
            if std::fs::create_dir_all(&dir_path).is_ok() {
                self.stream.write_all(format!("257 \"{}\" created\r\n", dir_path.display()).as_bytes())?;
                self.file_logger.lock().unwrap().log_mkdir(
                    self.current_user.as_deref().unwrap_or("anonymous"),
                    &self.remote_ip,
                    &dir_path.to_string_lossy(),
                    "FTP",
                );
                self.logger.lock().unwrap().client_action(
                    "FTP",
                    &format!("Created directory: {}", dirname),
                    &self.remote_ip,
                    self.current_user.as_deref(),
                    "MKDIR",
                );
            } else {
                self.stream.write_all(b"550 Create directory operation failed\r\n")?;
            }
        }
        Ok(())
    }

    pub fn cmd_rmd(&mut self, arg: Option<&str>) -> Result<()> {
        if !self.authenticated {
            self.stream.write_all(b"530 Not logged in\r\n")?;
            return Ok(());
        }

        {
            let users = self.user_manager.lock().unwrap();
            let user = self.current_user.as_ref().and_then(|u| users.get_user(u));

            if let Some(user) = user {
                if !user.permissions.can_rmdir {
                    self.stream.write_all(b"550 Permission denied\r\n")?;
                    return Ok(());
                }
            }
        }

        if let Some(dirname) = arg {
            let dir_path = safe_resolve_path(&self.cwd, &self.home_dir, dirname);
            if !dir_path.starts_with(&self.home_dir) {
                self.stream.write_all(b"550 Permission denied\r\n")?;
                return Ok(());
            }
            if std::fs::remove_dir_all(&dir_path).is_ok() {
                self.stream.write_all(b"250 Directory removed\r\n")?;
                self.file_logger.lock().unwrap().log_rmdir(
                    self.current_user.as_deref().unwrap_or("anonymous"),
                    &self.remote_ip,
                    &dir_path.to_string_lossy(),
                    "FTP",
                );
                self.logger.lock().unwrap().client_action(
                    "FTP",
                    &format!("Removed directory: {}", dirname),
                    &self.remote_ip,
                    self.current_user.as_deref(),
                    "RMDIR",
                );
            } else {
                self.stream.write_all(b"550 Remove directory operation failed\r\n")?;
            }
        }
        Ok(())
    }
}
