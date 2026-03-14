use anyhow::Result;
use std::io::Write;

use super::super::handler::FtpSession;

impl FtpSession {
    pub fn cmd_user(&mut self, arg: Option<&str>) -> Result<()> {
        if let Some(username) = arg {
            self.current_user = Some(username.to_string());
            self.stream.write_all(b"331 User name okay, need password\r\n")?;
        } else {
            self.stream.write_all(b"501 Syntax error in parameters or arguments\r\n")?;
        }
        Ok(())
    }

    pub fn cmd_pass(&mut self, arg: Option<&str>) -> Result<()> {
        if let Some(ref username) = self.current_user {
            let password = arg.unwrap_or("");
            let mut users = self.user_manager.lock().unwrap();

            if users.get_user(username).is_none() {
                let _ = users.reload(&std::path::PathBuf::from("/etc/wftpg/users.json"));
            }

            match users.authenticate(username, password) {
                Ok(true) => {
                    self.authenticated = true;
                    if let Some(user) = users.get_user(username) {
                        let home = std::path::PathBuf::from(&user.home_dir);
                        let home_canon = if home.exists() {
                            home.canonicalize().unwrap_or_else(|_| home.clone())
                        } else {
                            home.clone()
                        };
                        self.cwd = home_canon.to_string_lossy().to_string();
                        self.home_dir = home_canon.to_string_lossy().to_string();
                    }
                    self.stream.write_all(b"230 User logged in\r\n")?;
                    self.logger.lock().unwrap().client_action(
                        "FTP",
                        &format!("User {} logged in", username),
                        &self.remote_ip,
                        Some(username),
                        "LOGIN",
                    );
                }
                Ok(false) => {
                    self.logger.lock().unwrap().client_action(
                        "FTP",
                        &format!("Authentication failed for user {}", username),
                        &self.remote_ip,
                        Some(username),
                        "AUTH_FAIL",
                    );
                    self.stream.write_all(b"530 Not logged in, user cannot be authenticated\r\n")?;
                }
                Err(e) => {
                    self.logger.lock().unwrap().client_action(
                        "FTP",
                        &format!("Authentication error for user {}: {}", username, e),
                        &self.remote_ip,
                        Some(username),
                        "AUTH_ERROR",
                    );
                    self.stream.write_all(b"530 Not logged in\r\n")?;
                }
            }
        } else {
            self.stream.write_all(b"530 Please login with USER and PASS\r\n")?;
        }
        Ok(())
    }
}
