mod commands;
mod data_connection;
mod handler;
mod rate_limit;
mod utils;

use anyhow::Result;
use std::collections::HashMap;
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::core::config::Config;
use crate::core::logger::Logger;
use crate::core::users::UserManager;
use crate::core::file_logger::FileLogger;

use data_connection::PassiveListenerMap;
use handler::FtpSession;
use rate_limit::RateLimiter;

pub struct FtpServer {
    config: Arc<Mutex<Config>>,
    user_manager: Arc<Mutex<UserManager>>,
    logger: Arc<Mutex<Logger>>,
    file_logger: Arc<Mutex<FileLogger>>,
    running: Arc<Mutex<bool>>,
    listener: Arc<Mutex<Option<TcpListener>>>,
    passive_listeners: PassiveListenerMap,
    rate_limiter: Arc<RateLimiter>,
}

impl FtpServer {
    pub fn new(
        config: Arc<Mutex<Config>>,
        user_manager: Arc<Mutex<UserManager>>,
        logger: Arc<Mutex<Logger>>,
        file_logger: Arc<Mutex<FileLogger>>,
    ) -> Self {
        let rate_limiter = Arc::new(RateLimiter::new(10, 60, 100));
        
        FtpServer {
            config,
            user_manager,
            logger,
            file_logger,
            running: Arc::new(Mutex::new(false)),
            listener: Arc::new(Mutex::new(None)),
            passive_listeners: Arc::new(Mutex::new(HashMap::new())),
            rate_limiter,
        }
    }

    pub fn start(&self) -> Result<()> {
        let (bind_ip, ftp_port) = {
            let cfg = self.config.lock().unwrap();
            (cfg.server.bind_ip.clone(), cfg.server.ftp_port)
        };
        let bind_addr = format!("{}:{}", bind_ip, ftp_port);
        
        let listener = TcpListener::bind(&bind_addr)?;
        listener.set_nonblocking(true)?;
        
        {
            let mut running = self.running.lock().unwrap();
            *running = true;
        }
        
        {
            let mut listener_guard = self.listener.lock().unwrap();
            *listener_guard = Some(listener.try_clone()?);
        }

        self.logger.lock().unwrap().info("FTP", &format!("FTP server started on {}", bind_addr));

        let config = Arc::clone(&self.config);
        let user_manager = Arc::clone(&self.user_manager);
        let logger = Arc::clone(&self.logger);
        let file_logger = Arc::clone(&self.file_logger);
        let running = Arc::clone(&self.running);
        let passive_listeners = Arc::clone(&self.passive_listeners);
        let server_listener = Arc::clone(&self.listener);
        let rate_limiter = Arc::clone(&self.rate_limiter);

        std::thread::spawn(move || {
            loop {
                let is_running = *running.lock().unwrap();
                if !is_running {
                    break;
                }

                match listener.accept() {
                    Ok((stream, peer_addr)) => {
                        let config = Arc::clone(&config);
                        let user_manager = Arc::clone(&user_manager);
                        let logger_for_session = Arc::clone(&logger);
                        let logger_for_error = Arc::clone(&logger);
                        let file_logger = Arc::clone(&file_logger);
                        let passive_listeners = Arc::clone(&passive_listeners);
                        let rate_limiter = Arc::clone(&rate_limiter);

                        std::thread::spawn(move || {
                            match FtpSession::new(
                                stream,
                                config,
                                user_manager,
                                logger_for_session,
                                file_logger,
                                passive_listeners,
                                rate_limiter,
                            ) {
                                Ok(mut session) => {
                                    if let Err(e) = session.run() {
                                        logger_for_error.lock().unwrap().error(
                                            "FTP",
                                            &format!("Session error from {}: {}", peer_addr, e),
                                        );
                                    }
                                }
                                Err(e) => {
                                    logger_for_error.lock().unwrap().error(
                                        "FTP",
                                        &format!("Failed to create session from {}: {}", peer_addr, e),
                                    );
                                }
                            }
                        });
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(100));
                        continue;
                    }
                    Err(e) => {
                        let is_running = *running.lock().unwrap();
                        if !is_running {
                            break;
                        }
                        logger.lock().unwrap().error("FTP", &format!("Failed to accept connection: {}", e));
                    }
                }
            }
            
            {
                let mut listener_guard = server_listener.lock().unwrap();
                *listener_guard = None;
            }
            
            logger.lock().unwrap().info("FTP", "FTP server stopped");
        });

        Ok(())
    }

    pub fn stop(&self) {
        {
            let mut running = self.running.lock().unwrap();
            *running = false;
        }
        
        {
            let listener_guard = self.listener.lock().unwrap();
            if let Some(ref listener) = *listener_guard {
                let addr = listener.local_addr().ok();
                drop(listener_guard);
                
                if let Some(addr) = addr {
                    use std::net::TcpStream;
                    TcpStream::connect(addr).ok();
                }
            } else {
                drop(listener_guard);
            }
        }

        let mut listeners = self.passive_listeners.lock().unwrap();
        for (_, listener) in listeners.drain() {
            drop(listener);
        }
    }

    pub fn is_running(&self) -> bool {
        let listener_guard = self.listener.lock().unwrap();
        listener_guard.is_some()
    }

    pub fn get_connection_stats(&self) -> (usize, u32) {
        self.rate_limiter.get_stats()
    }
}
