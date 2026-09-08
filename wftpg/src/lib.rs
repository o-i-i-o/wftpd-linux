//! WFTP-GUI 前端库
//!
//! 仅做配置管理、服务控制与日志查看；所有对配置/用户文件的写入都通过
//! gRPC（UDS）交给后端 wftpd 完成，前端本地只持有共享类型（wftpd-common）。

pub mod communication;
pub mod ui;

use std::sync::{Arc, Mutex as StdMutex};

use wftpd_common::{Config, UserManager};

pub struct AppState {
    pub config: Arc<StdMutex<Config>>,
    pub user_manager: Arc<StdMutex<UserManager>>,
}

impl AppState {
    /// 创建前端本地状态（从 XDG 配置目录加载，作为 UI 的初始展示；
    /// 权威数据始终以后端为准）
    /// 创建前端本地状态（从 XDG 配置目录加载，作为 UI 的初始展示；
    /// 权威数据始终以后端为准）
    ///
    /// # Errors
    /// 配置文件或用户库读取/解析失败时返回错误
    pub fn new_for_gui() -> anyhow::Result<Self> {
        let config_path = Config::get_config_path();
        let config = Arc::new(StdMutex::new(Config::load(&config_path)?));

        let users_path = Config::get_users_path();
        let user_manager = Arc::new(StdMutex::new(UserManager::load(&users_path)?));

        Ok(AppState {
            config,
            user_manager,
        })
    }
}
