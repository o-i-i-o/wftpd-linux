//! WFTP-GUI 前端库
//!
//! 仅做配置管理、服务控制与日志查看；所有对配置/用户文件的写入都通过
//! gRPC（UDS）交给后端 wftpd 完成，前端本地只持有共享类型（wftpd-common）。

// 本 crate 为应用型项目内部代码（不作为库对外发布）：pedantic 的文档规范类
// lint（# Errors/# Panics 章节、#[must_use] 标注）与函数长度上限对内部 API
// 收益有限，统一在 crate 级关闭；具体取舍见仓库审计说明。
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::too_many_lines
)]
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
