//! WFTPD 共享基础库
//!
//! 提供前后端共用的基础设施：
//! - [`config`] / [`users`]：配置与用户管理（前后端共享同一份类型定义，避免格式漂移）
//! - [`paths`]：XDG 路径解析（用户态运行模型）
//! - 日志：tracing 初始化（含内存环形缓冲，供前端实时读取）
//! - [`server`]：FTP/SFTP 服务端公共工具（配额、限速、登录跟踪、路径安全）

// 本 crate 为应用型项目内部代码（不作为库对外发布）：pedantic 的文档规范类
// lint（# Errors/# Panics 章节、#[must_use] 标注）与函数长度上限对内部 API
// 收益有限，统一在 crate 级关闭；具体取舍见仓库审计说明。
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::too_many_lines
)]
pub mod config;
pub mod error;
pub mod file_logger;
pub mod logger;
pub mod paths;
pub mod server;
pub mod tracing_logger;
pub mod users;

pub use config::{Config, FtpConfig, LoggingConfig, SecurityConfig, SftpConfig};
pub use error::{WftpgError, WftpgResult};
pub use file_logger::{
    FileLogEntry, FileLogEntryJson, FileLogInfo, FileLogger, LogEntryJson, LogFileEntry,
};
pub use logger::{LogEntry, LogLevel, Logger};
pub use tracing_logger::{LogBuffer, init_simple, init_tracing, set_log_level};
pub use users::{Permissions, User, UserManager};
