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
    keys_dir: PathBuf,
}

impl SftpHandler {
    pub fn new(
        user_manager: Arc<StdMutex<UserManager>>,
        logger: Arc<StdMutex<Logger>>,
        file_logger: Arc<StdMutex<FileLogger>>,
        client_ip: String,
        users_path: std::path::PathBuf,
        keys_dir: PathBuf,
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
            keys_dir,
        }
    }

    fn log_client_action(
        &self,
        action: &str,
        message: &str,
        username: Option<&str>,
        log_type: &str,
    ) {
        if let Ok(mut log) = self.logger.try_lock() {
            log.client_action(action, message, &self.client_ip, username, log_type);
        }
    }

    fn log_warning(&self, message: &str) {
        if let Ok(mut log) = self.logger.try_lock() {
            log.warning("SFTP", message);
        }
    }

    fn log_info(&self, message: &str) {
        if let Ok(mut log) = self.logger.try_lock() {
            log.info("SFTP", message);
        }
    }

    async fn validate_and_set_home_dir(
        &mut self,
        user: &str,
        home_dir: &str,
    ) -> Result<PathBuf, String> {
        if home_dir.trim().is_empty() {
            return Err(format!("home directory not configured for user '{}'", user));
        }

        let home = PathBuf::from(home_dir);
        
        match tokio::fs::metadata(&home).await {
            Ok(metadata) => {
                if !metadata.is_dir() {
                    return Err(format!("home path '{}' is not a directory", home_dir));
                }
            }
            Err(e) => {
                return Err(format!("home directory '{}' does not exist or cannot be accessed: {}", home_dir, e));
            }
        }

        match tokio::fs::canonicalize(&home).await {
            Ok(home_canon) => {
                self.home_dir = Some(home_canon.to_string_lossy().to_string());
                Ok(home_canon)
            }
            Err(e) => {
                Err(format!("cannot canonicalize home directory '{}': {}", home.display(), e))
            }
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
            self.log_client_action(
                "SFTP",
                "Password auth failed: invalid username format",
                None,
                "AUTH_FAIL",
            );
            return Ok(server::Auth::Reject { 
                proceed_with_methods: None,
                partial_success: false,
            });
        }
        
        let auth_result = {
            match self.user_manager.try_lock() {
                Ok(mut users) => {
                    if users.get_user(user).is_none() {
                        let _ = users.reload(&self.users_path);
                    }
                    
                    match users.authenticate(user, password) {
                        Ok(true) => {
                            self.authenticated = true;
                            self.username = Some(user.to_string());
                            
                            users.get_user(user).map(|u| Ok(u.home_dir.clone()))
                        }
                        Ok(false) => Some(Err(false)),
                        Err(_) => Some(Err(true)),
                    }
                }
                Err(_) => {
                    self.log_warning("Failed to acquire user_manager lock during password auth");
                    return Ok(server::Auth::Reject { 
                        proceed_with_methods: None,
                        partial_success: false,
                    });
                }
            }
        };
        
        match auth_result {
            Some(Ok(home_dir)) => {
                match self.validate_and_set_home_dir(user, &home_dir).await {
                    Ok(_) => {
                        self.log_client_action(
                            "SFTP",
                            &format!("User {} logged in", user),
                            Some(user),
                            "LOGIN",
                        );
                        Ok(server::Auth::Accept)
                    }
                    Err(err_msg) => {
                        self.log_client_action(
                            "SFTP",
                            &format!("Login failed: {}", err_msg),
                            Some(user),
                            "LOGIN_FAIL",
                        );
                        Ok(server::Auth::Reject { 
                            proceed_with_methods: None,
                            partial_success: false,
                        })
                    }
                }
            }
            Some(Err(false)) => {
                self.log_client_action(
                    "SFTP",
                    &format!("Failed login attempt for user {}", user),
                    Some(user),
                    "AUTH_FAIL",
                );
                Ok(server::Auth::Reject { 
                    proceed_with_methods: None,
                    partial_success: false,
                })
            }
            Some(Err(true)) => {
                self.log_client_action(
                    "SFTP",
                    &format!("Authentication error for user {}", user),
                    Some(user),
                    "AUTH_ERROR",
                );
                Ok(server::Auth::Reject { 
                    proceed_with_methods: None,
                    partial_success: false,
                })
            }
            None => {
                self.log_client_action(
                    "SFTP",
                    &format!("User {} not found after authentication", user),
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
            self.log_client_action(
                "SFTP",
                "Public key auth failed: invalid username format",
                None,
                "AUTH_FAIL",
            );
            return Ok(server::Auth::Reject { 
                proceed_with_methods: None,
                partial_success: false,
            });
        }
        
        let (enabled, user_home_dir) = {
            match self.user_manager.try_lock() {
                Ok(users) => {
                    if let Some(u) = users.get_user(user) {
                        (u.enabled, u.home_dir.clone())
                    } else {
                        (false, String::new())
                    }
                }
                Err(_) => {
                    self.log_warning("Failed to acquire user_manager lock during public key auth");
                    return Ok(server::Auth::Reject { 
                        proceed_with_methods: None,
                        partial_success: false,
                    });
                }
            }
        };
        
        if !enabled {
            self.log_client_action(
                "SFTP",
                &format!("Public key auth failed for user {}: user not found or disabled", user),
                Some(user),
                "AUTH_FAIL",
            );
            return Ok(server::Auth::Reject { 
                proceed_with_methods: None,
                partial_success: false,
            });
        }
        
        let user_pubkey_path = self.keys_dir.join(format!("{}.pub", user));
        
        match tokio::fs::read_to_string(&user_pubkey_path).await {
            Ok(stored_key) => {
                match keys::parse_public_key_base64(stored_key.trim()) {
                    Ok(stored_pubkey) => {
                        if public_key == &stored_pubkey {
                            self.authenticated = true;
                            self.username = Some(user.to_string());
                            
                            match self.validate_and_set_home_dir(user, &user_home_dir).await {
                                Ok(_) => {
                                    self.log_client_action(
                                        "SFTP",
                                        &format!("User {} logged in via public key", user),
                                        Some(user),
                                        "LOGIN",
                                    );
                                    return Ok(server::Auth::Accept);
                                }
                                Err(err_msg) => {
                                    self.log_client_action(
                                        "SFTP",
                                        &format!("Login failed: {}", err_msg),
                                        Some(user),
                                        "LOGIN_FAIL",
                                    );
                                    return Ok(server::Auth::Reject { 
                                        proceed_with_methods: None,
                                        partial_success: false,
                                    });
                                }
                            }
                        }
                    }
                    Err(e) => {
                        self.log_client_action(
                            "SFTP",
                            &format!("Failed to parse stored public key for user {}: {}", user, e),
                            Some(user),
                            "AUTH_ERROR",
                        );
                    }
                }
            }
            Err(e) => {
                self.log_client_action(
                    "SFTP",
                    &format!("Failed to read public key file for user {}: {}", user, e),
                    Some(user),
                    "AUTH_ERROR",
                );
            }
        }

        self.log_client_action(
            "SFTP",
            &format!("Public key auth failed for user {}: key mismatch or not found", user),
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
        if name != "sftp" {
            self.log_warning(&format!("Unknown subsystem request: {}", name));
            let _ = session.channel_failure(channel);
            return Ok(());
        }

        if !self.authenticated {
            self.log_warning("SFTP subsystem request denied: not authenticated");
            let _ = session.channel_failure(channel);
            return Ok(());
        }

        let home_dir = match &self.home_dir {
            Some(h) => h.clone(),
            None => {
                self.log_warning("SFTP subsystem request denied: home_dir not set");
                let _ = session.channel_failure(channel);
                return Ok(());
            }
        };

        let username = self.username.clone();
        
        self.log_info(&format!("SFTP subsystem initialized for user: {:?}, home_dir: {}", username, home_dir));
        
        let _ = session.channel_success(channel);
        self.sftp_channel = Some(channel);
        
        self.sftp_state = Some(Arc::new(Mutex::new(SftpState::new(
            home_dir,
            username,
            Arc::clone(&self.user_manager),
            Arc::clone(&self.logger),
            Arc::clone(&self.file_logger),
            self.client_ip.clone(),
        ))));
        
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
                    self.log_warning(&format!(
                        "Channel EOF received with {} bytes of incomplete data in buffer, clearing",
                        buffer_len
                    ));
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
                    self.log_warning(&format!(
                        "Channel closed with {} bytes of incomplete data in buffer, clearing",
                        buffer_len
                    ));
                    sftp_state.buffer.clear();
                }
            }
            self.sftp_channel = None;
            self.sftp_state = None;
        }
        Ok(())
    }
}
