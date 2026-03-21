use anyhow::Result;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use crate::core::config::Config;
use crate::core::users::UserManager;
use crate::core::logger::Logger;

pub struct ConfigController {
    config: Arc<StdMutex<Config>>,
    user_manager: Arc<StdMutex<UserManager>>,
    logger: Arc<StdMutex<Logger>>,
}

impl ConfigController {
    pub fn new(
        config: Arc<StdMutex<Config>>,
        user_manager: Arc<StdMutex<UserManager>>,
        logger: Arc<StdMutex<Logger>>,
    ) -> Self {
        ConfigController {
            config,
            user_manager,
            logger,
        }
    }
    
    pub fn read_config(&self) -> Result<String> {
        crate::communication::dbus::read_config()
            .map_err(|e| anyhow::anyhow!("{}", e))
    }
    
    pub fn write_config(&self, content: &str) -> Result<()> {
        crate::communication::dbus::write_config(content)
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        
        if let Ok(mut log) = self.logger.lock() {
            log.info("CONFIG", "配置已保存");
        }
        
        Ok(())
    }
    
    pub fn read_users(&self) -> Result<String> {
        crate::communication::dbus::read_users()
            .map_err(|e| anyhow::anyhow!("{}", e))
    }
    
    pub fn write_users(&self, content: &str) -> Result<()> {
        crate::communication::dbus::write_users(content)
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        
        if let Ok(mut log) = self.logger.lock() {
            log.info("USERS", "用户配置已保存");
        }
        
        Ok(())
    }
    
    pub fn write_audit_log(&self, user: &str, action: &str, target: &str, details: &str) -> Result<()> {
        crate::communication::dbus::write_audit_log(user, action, target, details)
            .map_err(|e| anyhow::anyhow!("{}", e))
    }
    
    pub fn reload_config(&self) -> Result<()> {
        let config_path = Config::get_config_path();
        let new_config = Config::load(&config_path)?;
        
        if let Ok(mut cfg) = self.config.lock() {
            *cfg = new_config;
        }
        
        if let Ok(mut log) = self.logger.lock() {
            log.info("CONFIG", "配置已重新加载");
        }
        
        Ok(())
    }
    
    pub fn reload_users(&self) -> Result<()> {
        let users_path = Config::get_users_path();
        
        if let Ok(mut user_mgr) = self.user_manager.lock() {
            user_mgr.reload(&users_path)?;
        }
        
        if let Ok(mut log) = self.logger.lock() {
            log.info("USERS", "用户配置已重新加载");
        }
        
        Ok(())
    }
}
