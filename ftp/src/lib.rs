//! WFTPD FTP/FTPS 服务器（基于 libunftp）
//!
//! 职责划分：
//! - `auth`：libunftp Authenticator 桥——argon2 密码校验、匿名访问、IP 白/黑名单
//! - `storage`：libunftp StorageBackend——按用户主目录隔离，执行权限/配额/审计
//! - `server`：把 Config/UserManager/FileLogger 组装为 libunftp Server 并管理生命周期

// 本 crate 为应用型项目内部代码（不作为库对外发布）：pedantic 的文档规范类
// lint（# Errors/# Panics 章节、#[must_use] 标注）与函数长度上限对内部 API
// 收益有限，统一在 crate 级关闭；具体取舍见仓库审计说明。
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::too_many_lines
)]
pub mod auth;
pub mod server;
pub mod storage;

pub use server::FtpServer;
