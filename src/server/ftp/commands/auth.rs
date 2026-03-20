use anyhow::Result;
use std::path::Path;

use super::super::handler::FtpSession;

impl FtpSession {
    pub async fn cmd_user(&mut self, arg: Option<&str>) -> Result<()> {
        if let Some(username) = arg {
            if username.to_lowercase() == "anonymous" {
                let (allow_anonymous, anonymous_home) = {
                    let cfg = self.config.lock().unwrap();
                    (cfg.ftp.allow_anonymous, cfg.ftp.anonymous_home.clone())
                };
                
                if !allow_anonymous {
                    self.stream.write_all(b"530 Anonymous access not allowed\r\n").await?;
                    return Ok(());
                }
                
                match anonymous_home {
                    Some(home) if !home.trim().is_empty() => {
                        let home_path = Path::new(&home);
                        if !home_path.exists() {
                            self.logger.lock().unwrap().client_action(
                                "FTP",
                                &format!("Anonymous login failed: anonymous home directory '{}' does not exist", home),
                                &self.remote_ip,
                                Some("anonymous"),
                                "LOGIN_FAIL",
                            );
                            self.stream.write_all(b"530 Login failed: anonymous home directory does not exist\r\n").await?;
                            return Ok(());
                        }
                        if !home_path.is_dir() {
                            self.logger.lock().unwrap().client_action(
                                "FTP",
                                &format!("Anonymous login failed: anonymous home path '{}' is not a directory", home),
                                &self.remote_ip,
                                Some("anonymous"),
                                "LOGIN_FAIL",
                            );
                            self.stream.write_all(b"530 Login failed: anonymous home path is not a directory\r\n").await?;
                            return Ok(());
                        }
                        
                        let home_canon = match home_path.canonicalize() {
                            Ok(c) => c,
                            Err(e) => {
                                self.logger.lock().unwrap().client_action(
                                    "FTP",
                                    &format!("Anonymous login failed: cannot canonicalize anonymous home directory '{}': {}", home, e),
                                    &self.remote_ip,
                                    Some("anonymous"),
                                    "LOGIN_FAIL",
                                );
                                self.stream.write_all(b"530 Login failed: cannot access anonymous home directory\r\n").await?;
                                return Ok(());
                            }
                        };
                        
                        self.current_user = Some("anonymous".to_string());
                        self.cwd = home_canon.to_string_lossy().to_string();
                        self.home_dir = home_canon.to_string_lossy().to_string();
                        self.authenticated = true;
                        self.stream.write_all(b"230 Anonymous login successful\r\n").await?;
                        self.logger.lock().unwrap().client_action(
                            "FTP",
                            "Anonymous user logged in",
                            &self.remote_ip,
                            Some("anonymous"),
                            "LOGIN",
                        );
                    }
                    _ => {
                        self.logger.lock().unwrap().client_action(
                            "FTP",
                            "Anonymous login failed: anonymous home directory not configured",
                            &self.remote_ip,
                            Some("anonymous"),
                            "LOGIN_FAIL",
                        );
                        self.stream.write_all(b"530 Anonymous login failed: anonymous home directory not configured\r\n").await?;
                    }
                }
            } else {
                self.current_user = Some(username.to_string());
                self.authenticated = false;
                self.stream.write_all(b"331 User name okay, need password\r\n").await?;
            }
        } else {
            self.stream.write_all(b"501 Syntax error in parameters or arguments\r\n").await?;
        }
        Ok(())
    }

    pub async fn cmd_pass(&mut self, arg: Option<&str>) -> Result<()> {
        if let Some(ref username) = self.current_user {
            if username.to_lowercase() == "anonymous" {
                self.stream.write_all(b"230 Already logged in as anonymous\r\n").await?;
                return Ok(());
            }
            
            let password = arg.unwrap_or("");
            
            let (_user_data, should_reload) = {
                let users = self.user_manager.lock().unwrap();
                (users.get_user(username).cloned(), users.get_user(username).is_none())
            };
            
            if should_reload {
                let mut users = self.user_manager.lock().unwrap();
                let _ = users.reload(&std::path::PathBuf::from("/etc/wftpg/users.json"));
            }
            
            let auth_result = {
                let mut users = self.user_manager.lock().unwrap();
                users.authenticate(username, password)
            };
            
            match auth_result {
                Ok(true) => {
                    let user_info = {
                        let users = self.user_manager.lock().unwrap();
                        users.get_user(username).cloned()
                    };
                    
                    if let Some(user) = user_info {
                        if user.home_dir.trim().is_empty() {
                            self.logger.lock().unwrap().client_action(
                                "FTP",
                                &format!("Login failed: home directory not configured for user '{}'", username),
                                &self.remote_ip,
                                Some(username),
                                "LOGIN_FAIL",
                            );
                            self.stream.write_all(b"530 Login failed: home directory not configured\r\n").await?;
                            self.authenticated = false;
                            return Ok(());
                        }
                        
                        let home = std::path::PathBuf::from(&user.home_dir);
                        if !home.exists() {
                            self.logger.lock().unwrap().client_action(
                                "FTP",
                                &format!("Login failed: home directory '{}' does not exist", user.home_dir),
                                &self.remote_ip,
                                Some(username),
                                "LOGIN_FAIL",
                            );
                            self.stream.write_all(b"530 Login failed: home directory does not exist\r\n").await?;
                            self.authenticated = false;
                            return Ok(());
                        }
                        if !home.is_dir() {
                            self.logger.lock().unwrap().client_action(
                                "FTP",
                                &format!("Login failed: home path '{}' is not a directory", user.home_dir),
                                &self.remote_ip,
                                Some(username),
                                "LOGIN_FAIL",
                            );
                            self.stream.write_all(b"530 Login failed: home path is not a directory\r\n").await?;
                            self.authenticated = false;
                            return Ok(());
                        }
                        let home_canon = match home.canonicalize() {
                            Ok(c) => c,
                            Err(e) => {
                                self.logger.lock().unwrap().client_action(
                                    "FTP",
                                    &format!("Login failed: cannot canonicalize home directory '{}': {}", home.display(), e),
                                    &self.remote_ip,
                                    Some(username),
                                    "LOGIN_FAIL",
                                );
                                self.stream.write_all(b"530 Login failed: cannot access home directory\r\n").await?;
                                self.authenticated = false;
                                return Ok(());
                            }
                        };
                        self.cwd = home_canon.to_string_lossy().to_string();
                        self.home_dir = home_canon.to_string_lossy().to_string();
                        self.authenticated = true;
                        self.stream.write_all(b"230 User logged in\r\n").await?;
                        self.logger.lock().unwrap().client_action(
                            "FTP",
                            &format!("User {} logged in", username),
                            &self.remote_ip,
                            Some(username),
                            "LOGIN",
                        );
                    } else {
                        self.authenticated = false;
                        self.logger.lock().unwrap().warning(
                            "FTP",
                            &format!("User {} authenticated but not found in user list", username),
                        );
                        self.stream.write_all(b"530 Login failed: user data not found\r\n").await?;
                    }
                }
                Ok(false) => {
                    self.authenticated = false;
                    self.logger.lock().unwrap().client_action(
                        "FTP",
                        &format!("Authentication failed for user {}", username),
                        &self.remote_ip,
                        Some(username),
                        "AUTH_FAIL",
                    );
                    self.stream.write_all(b"530 Not logged in, user cannot be authenticated\r\n").await?;
                }
                Err(e) => {
                    self.authenticated = false;
                    self.logger.lock().unwrap().client_action(
                        "FTP",
                        &format!("Authentication error for user {}: {}", username, e),
                        &self.remote_ip,
                        Some(username),
                        "AUTH_ERROR",
                    );
                    self.stream.write_all(b"530 Not logged in\r\n").await?;
                }
            }
        } else {
            self.stream.write_all(b"530 Please login with USER and PASS\r\n").await?;
        }
        Ok(())
    }
}
