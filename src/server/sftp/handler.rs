use anyhow::Result;
use russh::*;
use russh::keys::*;
use russh::server::Msg;
use russh::ChannelId;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use tokio::sync::Mutex;

use super::state::SftpState;
use crate::core::logger::Logger;
use crate::core::users::UserManager;
use crate::core::file_logger::FileLogger;
use crate::server::common::utils::is_safe_username;

pub struct SftpHandler {
    user_manager: Arc<StdMutex<UserManager>>,
    logger: Arc<StdMutex<Logger>>,
    file_logger: Arc<StdMutex<FileLogger>>,
    authenticated: bool,
    username: Option<String>,
    home_dir: Option<String>,
    sftp_channel: Option<ChannelId>,
    sftp_state: Option<Arc<Mutex<SftpState>>>,
    client_ip: String,
    users_path: std::path::PathBuf,
}

impl SftpHandler {
    pub fn new(
        user_manager: Arc<StdMutex<UserManager>>,
        logger: Arc<StdMutex<Logger>>,
        file_logger: Arc<StdMutex<FileLogger>>,
        client_ip: String,
        users_path: std::path::PathBuf,
    ) -> Self {
        SftpHandler {
            user_manager,
            logger,
            file_logger,
            authenticated: false,
            username: None,
            home_dir: None,
            sftp_channel: None,
            sftp_state: None,
            client_ip,
            users_path,
        }
    }
}

impl russh::server::Handler for SftpHandler {
    type Error = anyhow::Error;

    async fn auth_password(
        &mut self,
        user: &str,
        password: &str,
    ) -> Result<server::Auth, Self::Error> {
        if !is_safe_username(user) {
            self.logger.lock().unwrap().client_action(
                "SFTP",
                "Password auth failed: invalid username format",
                &self.client_ip,
                None,
                "AUTH_FAIL",
            );
            return Ok(server::Auth::Reject { 
                proceed_with_methods: None,
                partial_success: false,
            });
        }
        
        let mut users = self.user_manager.lock().unwrap();
        
        if users.get_user(user).is_none() {
            let _ = users.reload(&self.users_path);
        }
        
        match users.authenticate(user, password) {
            Ok(true) => {
                self.authenticated = true;
                self.username = Some(user.to_string());
                
                if let Some(u) = users.get_user(user) {
                    if u.home_dir.trim().is_empty() {
                        self.logger.lock().unwrap().client_action(
                            "SFTP",
                            &format!("Login failed: home directory not configured for user '{}'", user),
                            &self.client_ip,
                            Some(user),
                            "LOGIN_FAIL",
                        );
                        return Ok(server::Auth::Reject { 
                            proceed_with_methods: None,
                            partial_success: false,
                        });
                    }
                    
                    let home = std::path::PathBuf::from(&u.home_dir);
                    if !home.exists() {
                        self.logger.lock().unwrap().client_action(
                            "SFTP",
                            &format!("Login failed: home directory '{}' does not exist", u.home_dir),
                            &self.client_ip,
                            Some(user),
                            "LOGIN_FAIL",
                        );
                        return Ok(server::Auth::Reject { 
                            proceed_with_methods: None,
                            partial_success: false,
                        });
                    }
                    if !home.is_dir() {
                        self.logger.lock().unwrap().client_action(
                            "SFTP",
                            &format!("Login failed: home path '{}' is not a directory", u.home_dir),
                            &self.client_ip,
                            Some(user),
                            "LOGIN_FAIL",
                        );
                        return Ok(server::Auth::Reject { 
                            proceed_with_methods: None,
                            partial_success: false,
                        });
                    }
                    let home_canon = match home.canonicalize() {
                        Ok(c) => c,
                        Err(e) => {
                            self.logger.lock().unwrap().client_action(
                                "SFTP",
                                &format!("Login failed: cannot canonicalize home directory '{}': {}", home.display(), e),
                                &self.client_ip,
                                Some(user),
                                "LOGIN_FAIL",
                            );
                            return Ok(server::Auth::Reject { 
                                proceed_with_methods: None,
                                partial_success: false,
                            });
                        }
                    };
                    self.home_dir = Some(home_canon.to_string_lossy().to_string());
                }

                self.logger.lock().unwrap().client_action(
                    "SFTP",
                    &format!("User {} logged in", user),
                    &self.client_ip,
                    Some(user),
                    "LOGIN",
                );

                Ok(server::Auth::Accept)
            }
            Ok(false) => {
                self.logger.lock().unwrap().client_action(
                    "SFTP",
                    &format!("Failed login attempt for user {}", user),
                    &self.client_ip,
                    Some(user),
                    "AUTH_FAIL",
                );
                Ok(server::Auth::Reject { 
                    proceed_with_methods: None,
                    partial_success: false,
                })
            }
            Err(e) => {
                self.logger.lock().unwrap().client_action(
                    "SFTP",
                    &format!("Authentication error for user {}: {}", user, e),
                    &self.client_ip,
                    Some(user),
                    "AUTH_ERROR",
                );
                Ok(server::Auth::Reject { 
                    proceed_with_methods: None,
                    partial_success: false,
                })
            }
        }
    }

    async fn auth_publickey(
        &mut self,
        user: &str,
        public_key: &PublicKey,
    ) -> Result<server::Auth, Self::Error> {
        if !is_safe_username(user) {
            self.logger.lock().unwrap().client_action(
                "SFTP",
                "Public key auth failed: invalid username format",
                &self.client_ip,
                None,
                "AUTH_FAIL",
            );
            return Ok(server::Auth::Reject { 
                proceed_with_methods: None,
                partial_success: false,
            });
        }
        
        let (enabled, user_pubkey_path) = {
            let users = self.user_manager.lock().unwrap();
            if let Some(u) = users.get_user(user) {
                let safe_path = PathBuf::from("/etc/wftpg/keys")
                    .join(format!("{}.pub", user));
                (u.enabled, safe_path.to_string_lossy().to_string())
            } else {
                (false, String::new())
            }
        };
        
        if !enabled {
            self.logger.lock().unwrap().client_action(
                "SFTP",
                &format!("Public key auth failed for user {}: user not found or disabled", user),
                &self.client_ip,
                Some(user),
                "AUTH_FAIL",
            );
            return Ok(server::Auth::Reject { 
                proceed_with_methods: None,
                partial_success: false,
            });
        }
        
        if let Ok(stored_key) = tokio::fs::read_to_string(&user_pubkey_path).await
            && let Ok(stored_pubkey) = keys::parse_public_key_base64(stored_key.trim())
                && public_key == &stored_pubkey {
                    self.authenticated = true;
                    self.username = Some(user.to_string());
                    
                    let users = self.user_manager.lock().unwrap();
                    if let Some(u) = users.get_user(user) {
                        if u.home_dir.trim().is_empty() {
                            self.logger.lock().unwrap().client_action(
                                "SFTP",
                                &format!("Login failed: home directory not configured for user '{}'", user),
                                &self.client_ip,
                                Some(user),
                                "LOGIN_FAIL",
                            );
                            return Ok(server::Auth::Reject { 
                                proceed_with_methods: None,
                                partial_success: false,
                            });
                        }
                        
                        let home = std::path::PathBuf::from(&u.home_dir);
                        if !home.exists() {
                            self.logger.lock().unwrap().client_action(
                                "SFTP",
                                &format!("Login failed: home directory '{}' does not exist", u.home_dir),
                                &self.client_ip,
                                Some(user),
                                "LOGIN_FAIL",
                            );
                            return Ok(server::Auth::Reject { 
                                proceed_with_methods: None,
                                partial_success: false,
                            });
                        }
                        if !home.is_dir() {
                            self.logger.lock().unwrap().client_action(
                                "SFTP",
                                &format!("Login failed: home path '{}' is not a directory", u.home_dir),
                                &self.client_ip,
                                Some(user),
                                "LOGIN_FAIL",
                            );
                            return Ok(server::Auth::Reject { 
                                proceed_with_methods: None,
                                partial_success: false,
                            });
                        }
                        let home_canon = match home.canonicalize() {
                            Ok(c) => c,
                            Err(e) => {
                                self.logger.lock().unwrap().client_action(
                                    "SFTP",
                                    &format!("Login failed: cannot canonicalize home directory '{}': {}", home.display(), e),
                                    &self.client_ip,
                                    Some(user),
                                    "LOGIN_FAIL",
                                );
                                return Ok(server::Auth::Reject { 
                                    proceed_with_methods: None,
                                    partial_success: false,
                                });
                            }
                        };
                        self.home_dir = Some(home_canon.to_string_lossy().to_string());
                    }

                    self.logger.lock().unwrap().client_action(
                        "SFTP",
                        &format!("User {} logged in via public key", user),
                        &self.client_ip,
                        Some(user),
                        "LOGIN",
                    );

                    return Ok(server::Auth::Accept);
                }

        self.logger.lock().unwrap().client_action(
            "SFTP",
            &format!("Public key auth failed for user {}: key mismatch or not found", user),
            &self.client_ip,
            Some(user),
            "AUTH_FAIL",
        );

        Ok(server::Auth::Reject { 
            proceed_with_methods: None,
            partial_success: false,
        })
    }

    async fn channel_open_session(
        &mut self,
        _channel: Channel<Msg>,
        _session: &mut server::Session,
    ) -> Result<bool, Self::Error> {
        Ok(self.authenticated)
    }

    async fn subsystem_request(
        &mut self,
        channel: ChannelId,
        name: &str,
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        if name == "sftp" && self.authenticated {
            let _ = session.channel_success(channel);
            
            self.sftp_channel = Some(channel);
            
            let home_dir = self.home_dir.clone().unwrap_or_else(|| "/tmp".to_string());
            let username = self.username.clone();
            
            self.logger.lock().unwrap().info(
                "SFTP",
                &format!("SFTP subsystem initialized for user: {:?}, home_dir: {}", username, home_dir),
            );
            
            self.sftp_state = Some(Arc::new(Mutex::new(SftpState::new(
                home_dir,
                username,
                Arc::clone(&self.user_manager),
                Arc::clone(&self.logger),
                Arc::clone(&self.file_logger),
                self.client_ip.clone(),
            ))));
        } else {
            self.logger.lock().unwrap().warning(
                "SFTP",
                &format!("SFTP subsystem request denied. authenticated: {}, name: {}", self.authenticated, name),
            );
            let _ = session.channel_failure(channel);
        }
        Ok(())
    }

    async fn pty_request(
        &mut self,
        _channel: ChannelId,
        _term: &str,
        _col_width: u32,
        _row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(Pty, u32)],
        _session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn shell_request(
        &mut self,
        _channel: ChannelId,
        _session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        if self.sftp_channel == Some(channel)
            && let Some(state) = &self.sftp_state {
                let response = {
                    let mut sftp_state = state.lock().await;
                    sftp_state.process_sftp_data(data).await
                };
                
                if let Ok(resp) = response
                    && !resp.is_empty() {
                        let _ = session.data(channel, resp);
                    }
            }
        Ok(())
    }

    async fn channel_eof(
        &mut self,
        channel: ChannelId,
        _session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        if self.sftp_channel == Some(channel)
            && let Some(state) = &self.sftp_state {
                let mut sftp_state = state.lock().await;
                let buffer_len = sftp_state.buffer.len();
                if buffer_len > 0 {
                    self.logger.lock().unwrap().warning(
                        "SFTP",
                        &format!(
                            "Channel EOF received with {} bytes of incomplete data in buffer, clearing",
                            buffer_len
                        ),
                    );
                    sftp_state.buffer.clear();
                }
            }
        Ok(())
    }

    async fn channel_close(
        &mut self,
        channel: ChannelId,
        _session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        if self.sftp_channel == Some(channel) {
            if let Some(state) = &self.sftp_state {
                let mut sftp_state = state.lock().await;
                let buffer_len = sftp_state.buffer.len();
                if buffer_len > 0 {
                    self.logger.lock().unwrap().warning(
                        "SFTP",
                        &format!(
                            "Channel closed with {} bytes of incomplete data in buffer, clearing",
                            buffer_len
                        ),
                    );
                    sftp_state.buffer.clear();
                }
            }
            self.sftp_channel = None;
            self.sftp_state = None;
        }
        Ok(())
    }
}
