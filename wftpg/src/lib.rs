pub mod core;
pub mod communication;
pub mod ui;

use std::sync::{Arc, Mutex};

use crate::core::config::Config;
use crate::core::users::UserManager;

pub struct AppState {
    pub config: Arc<Mutex<Config>>,
    pub user_manager: Arc<Mutex<UserManager>>,
}

impl AppState {
    /// 创建 AppState（前端使用）
    pub fn new_for_gui() -> anyhow::Result<Self> {
        let config_path = Config::get_config_path();
        let config = Arc::new(Mutex::new(Config::load(&config_path)?));
        
        let users_path = Config::get_users_path();
        let user_manager = Arc::new(Mutex::new(UserManager::load(&users_path)?));
        
        Ok(AppState {
            config,
            user_manager,
        })
    }
}
