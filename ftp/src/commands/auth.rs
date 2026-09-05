use anyhow::Result;
use std::path::Path;
use tracing::{debug, error, info, warn};

use super::super::handler::FtpSession;

impl FtpSession {
    pub async fn cmd_user(&mut self, arg: Option<&str>) -> Result<()> {
        if self.is_login_banned() {
            self.stream
                .write_all(b"530 Too many failed login attempts, please try again later\r\n")
                .await?;
            return Ok(());
        }

        if let Some(username) = arg {
            if username.to_lowercase() == "anonymous" {
                let (allow_anonymous, anonymous_home) = {
                    let cfg = self.config.lock().unwrap();
                    (cfg.ftp.allow_anonymous, cfg.ftp.anonymous_home.clone())
                };

                if !allow_anonymous {
                    self.stream
                        .write_all(b"530 Anonymous access not allowed\r\n")
                        .await?;
                    return Ok(());
                }

                match anonymous_home {
                    Some(home) if !home.trim().is_empty() => {
                        let home_path = Path::new(&home);
                        if !home_path.exists() {
                            warn!(
                                client_ip = %self.remote_ip,
                                home = %home,
                                "FTP 匿名登录失败：匿名主目录不存在"
                            );
                            self.stream.write_all(b"530 Login failed: anonymous home directory does not exist\r\n").await?;
                            return Ok(());
                        }
                        if !home_path.is_dir() {
                            warn!(
                                client_ip = %self.remote_ip,
                                home = %home,
                                "FTP 匿名登录失败：匿名主目录不是目录"
                            );
                            self.stream
                                .write_all(
                                    b"530 Login failed: anonymous home path is not a directory\r\n",
                                )
                                .await?;
                            return Ok(());
                        }

                        let home_canon = match home_path.canonicalize() {
                            Ok(c) => c,
                            Err(e) => {
                                warn!(
                                    client_ip = %self.remote_ip,
                                    home = %home,
                                    error = %e,
                                    "FTP 匿名登录失败：无法解析匿名主目录"
                                );
                                self.stream.write_all(b"530 Login failed: cannot access anonymous home directory\r\n").await?;
                                return Ok(());
                            }
                        };

                        self.current_user = Some("anonymous".to_string());
                        self.cwd = home_canon.to_string_lossy().to_string();
                        self.home_dir = home_canon.to_string_lossy().to_string();
                        self.authenticated = true;
                        self.login_tracker.clear_attempts(&self.remote_ip);
                        self.stream
                            .write_all(b"230 Anonymous login successful\r\n")
                            .await?;
                        // 使用 tracing 记录日志
                        info!(
                            username = "anonymous",
                            client_ip = %self.remote_ip,
                            "FTP 匿名用户登录成功"
                        );
                    }
                    _ => {
                        warn!(
                            client_ip = %self.remote_ip,
                            "FTP 匿名登录失败：未配置匿名主目录"
                        );
                        self.stream.write_all(b"530 Anonymous login failed: anonymous home directory not configured\r\n").await?;
                    }
                }
            } else {
                self.current_user = Some(username.to_string());
                self.authenticated = false;
                self.stream
                    .write_all(b"331 User name okay, need password\r\n")
                    .await?;
            }
        } else {
            self.stream
                .write_all(b"501 Syntax error in parameters or arguments\r\n")
                .await?;
        }
        Ok(())
    }

    pub async fn cmd_pass(&mut self, arg: Option<&str>) -> Result<()> {
        if self.is_login_banned() {
            self.stream
                .write_all(b"530 Too many failed login attempts, please try again later\r\n")
                .await?;
            return Ok(());
        }

        if let Some(ref username) = self.current_user {
            if username.to_lowercase() == "anonymous" {
                self.stream
                    .write_all(b"230 Already logged in as anonymous\r\n")
                    .await?;
                return Ok(());
            }

            let password = arg.unwrap_or("");

            // Always reload users from disk to ensure latest data
            let users_path = wftpd_common::paths::users_path();
            debug!(user = %username, ip = %self.remote_ip, "[FTP AUTH] 用户尝试登录");
            debug!(users_path = ?users_path, "[FTP AUTH] 重新加载用户配置文件");
            {
                let mut users = self.user_manager.lock().unwrap();
                if let Err(e) = users.reload(&users_path) {
                    error!(error = %e, "[FTP AUTH] 重新加载用户配置失败");
                }
                let user_count = users.get_users().len();
                debug!(user_count = user_count, "[FTP AUTH] 当前内存中用户数");

                // 检查用户是否存在
                if let Some(user) = users.get_user(username) {
                    debug!(
                        user = %username,
                        enabled = user.enabled,
                        home_dir = %user.home_dir,
                        "[FTP AUTH] 找到用户"
                    );
                } else {
                    warn!(user = %username, "[FTP AUTH] 未找到用户");
                }
            }

            let auth_result = {
                let mut users = self.user_manager.lock().unwrap();
                debug!(user = %username, "[FTP AUTH] 开始验证密码");
                users.authenticate(username, password)
            };

            match auth_result {
                Ok(true) => {
                    info!(user = %username, ip = %self.remote_ip, "[FTP AUTH] 用户认证成功");
                    let user_info = {
                        let users = self.user_manager.lock().unwrap();
                        users.get_user(username).cloned()
                    };

                    if let Some(user) = user_info {
                        if user.home_dir.trim().is_empty() {
                            self.file_logger
                                .lock()
                                .unwrap()
                                .log(wftpd_common::FileLogInfo {
                                    username,
                                    client_ip: &self.remote_ip,
                                    operation: "LOGIN_FAIL",
                                    file_path: "-",
                                    file_size: 0,
                                    protocol: "FTP",
                                    success: false,
                                    message: "Login failed: home directory not configured",
                                });
                            self.stream
                                .write_all(b"530 Login failed: home directory not configured\r\n")
                                .await?;
                            self.authenticated = false;
                            return Ok(());
                        }

                        let home = std::path::PathBuf::from(&user.home_dir);
                        if !home.exists() {
                            self.file_logger
                                .lock()
                                .unwrap()
                                .log(wftpd_common::FileLogInfo {
                                    username,
                                    client_ip: &self.remote_ip,
                                    operation: "LOGIN_FAIL",
                                    file_path: "-",
                                    file_size: 0,
                                    protocol: "FTP",
                                    success: false,
                                    message: "Login failed: home directory does not exist",
                                });
                            self.stream
                                .write_all(b"530 Login failed: home directory does not exist\r\n")
                                .await?;
                            self.authenticated = false;
                            return Ok(());
                        }
                        if !home.is_dir() {
                            self.file_logger
                                .lock()
                                .unwrap()
                                .log(wftpd_common::FileLogInfo {
                                    username,
                                    client_ip: &self.remote_ip,
                                    operation: "LOGIN_FAIL",
                                    file_path: "-",
                                    file_size: 0,
                                    protocol: "FTP",
                                    success: false,
                                    message: "Login failed: home path is not a directory",
                                });
                            self.stream
                                .write_all(b"530 Login failed: home path is not a directory\r\n")
                                .await?;
                            self.authenticated = false;
                            return Ok(());
                        }
                        let home_canon = match home.canonicalize() {
                            Ok(c) => c,
                            Err(e) => {
                                self.file_logger
                                    .lock()
                                    .unwrap()
                                    .log(wftpd_common::FileLogInfo {
                                        username,
                                        client_ip: &self.remote_ip,
                                        operation: "LOGIN_FAIL",
                                        file_path: "-",
                                        file_size: 0,
                                        protocol: "FTP",
                                        success: false,
                                        message: &format!(
                                            "Login failed: cannot canonicalize home directory: {}",
                                            e
                                        ),
                                    });
                                self.stream
                                    .write_all(
                                        b"530 Login failed: cannot access home directory\r\n",
                                    )
                                    .await?;
                                self.authenticated = false;
                                return Ok(());
                            }
                        };
                        self.cwd = home_canon.to_string_lossy().to_string();
                        self.home_dir = home_canon.to_string_lossy().to_string();
                        self.authenticated = true;
                        self.login_tracker.clear_attempts(&self.remote_ip);
                        self.stream.write_all(b"230 User logged in\r\n").await?;
                        self.file_logger
                            .lock()
                            .unwrap()
                            .log(wftpd_common::FileLogInfo {
                                username,
                                client_ip: &self.remote_ip,
                                operation: "LOGIN",
                                file_path: "-",
                                file_size: 0,
                                protocol: "FTP",
                                success: true,
                                message: "User logged in",
                            });
                    } else {
                        error!(user = %username, "[FTP AUTH] 用户认证成功但未找到用户数据");
                        self.authenticated = false;
                        warn!("User {} authenticated but not found in user list", username);
                        self.stream
                            .write_all(b"530 Login failed: user data not found\r\n")
                            .await?;
                    }
                }
                Ok(false) | Err(_) => {
                    info!(user = %username, ip = %self.remote_ip, "[FTP AUTH] 用户认证失败");
                    self.authenticated = false;
                    let remaining = self.login_tracker.get_remaining_attempts(&self.remote_ip);
                    if !self.login_tracker.check_and_record_failure(&self.remote_ip) {
                        warn!(
                            "IP {} banned due to too many failed login attempts",
                            self.remote_ip
                        );
                        self.stream.write_all(b"530 Too many failed login attempts, you are temporarily banned\r\n").await?;
                    } else {
                        self.file_logger
                            .lock()
                            .unwrap()
                            .log(wftpd_common::FileLogInfo {
                                username,
                                client_ip: &self.remote_ip,
                                operation: "AUTH_FAIL",
                                file_path: "-",
                                file_size: 0,
                                protocol: "FTP",
                                success: false,
                                message: &format!(
                                    "Authentication failed ({} attempts remaining)",
                                    remaining.saturating_sub(1)
                                ),
                            });
                        self.stream
                            .write_all(b"530 Not logged in, user cannot be authenticated\r\n")
                            .await?;
                    }
                }
            }
        } else {
            self.stream
                .write_all(b"530 Please login with USER and PASS\r\n")
                .await?;
        }
        Ok(())
    }
}
