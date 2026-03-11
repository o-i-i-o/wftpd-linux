use std::sync::{Arc, Mutex};
use std::path::PathBuf;

mod config;
mod users;
mod logger;
mod ftp_server;
mod sftp_server;
mod service;

use config::Config;
use users::UserManager;
use logger::Logger;
use ftp_server::FtpServer;
use sftp_server::SftpServer;

struct ServiceState {
    config: Arc<Mutex<Config>>,
    user_manager: Arc<Mutex<UserManager>>,
    logger: Arc<Mutex<Logger>>,
    ftp_server: Option<FtpServer>,
    sftp_server: Option<SftpServer>,
}

impl ServiceState {
    fn new() -> Self {
        let config_path = Config::get_config_path();
        let users_path = Config::get_users_path();
        
        let config = Config::load(&config_path).unwrap_or_else(|_| Config::default());
        let user_manager = UserManager::load(&users_path).unwrap_or_else(|_| UserManager::new());
        
        let mut logger = Logger::new(
            &config.logging.log_dir,
            config.logging.max_log_size,
            config.logging.max_log_files,
        );
        
        let _ = logger.init();
        logger.info("SERVICE", "WFTPG service starting");
        
        ServiceState {
            config: Arc::new(Mutex::new(config)),
            user_manager: Arc::new(Mutex::new(user_manager)),
            logger: Arc::new(Mutex::new(logger)),
            ftp_server: None,
            sftp_server: None,
        }
    }
    
    fn start_servers(&mut self) -> anyhow::Result<()> {
        let ftp_enabled = self.config.lock().unwrap().ftp.enabled;
        let sftp_enabled = self.config.lock().unwrap().sftp.enabled;
        
        if ftp_enabled {
            let config = Arc::clone(&self.config);
            let user_manager = Arc::clone(&self.user_manager);
            let logger = Arc::clone(&self.logger);
            
            let server = FtpServer::new(config, user_manager, logger);
            server.start()?;
            self.ftp_server = Some(server);
            
            self.logger.lock().unwrap().info("SERVICE", "FTP server started");
        }
        
        if sftp_enabled {
            let config = Arc::clone(&self.config);
            let user_manager = Arc::clone(&self.user_manager);
            let logger = Arc::clone(&self.logger);
            
            let server = SftpServer::new(config, user_manager, logger);
            server.start()?;
            self.sftp_server = Some(server);
            
            self.logger.lock().unwrap().info("SERVICE", "SFTP server started");
        }
        
        Ok(())
    }
    
    fn stop_servers(&mut self) {
        if let Some(server) = self.ftp_server.take() {
            server.stop();
            self.logger.lock().unwrap().info("SERVICE", "FTP server stopped");
        }
        
        if let Some(server) = self.sftp_server.take() {
            server.stop();
            self.logger.lock().unwrap().info("SERVICE", "SFTP server stopped");
        }
    }
}

fn main() -> anyhow::Result<()> {
    let mut state = ServiceState::new();
    
    state.start_servers()?;
    
    let running = Arc::new(Mutex::new(true));
    let r = running.clone();
    
    ctrlc::set_handler(move || {
        *r.lock().unwrap() = false;
    }).expect("Error setting Ctrl-C handler");
    
    while *running.lock().unwrap() {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    
    state.stop_servers();
    state.logger.lock().unwrap().info("SERVICE", "WFTPG service stopped");
    
    Ok(())
}
