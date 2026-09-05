//! WFTPD FTP/FTPS 服务器（基于 libunftp）
//!
//! 职责划分：
//! - `auth`：libunftp Authenticator 桥——argon2 密码校验、匿名访问、IP 白/黑名单
//! - `storage`：libunftp StorageBackend——按用户主目录隔离，执行权限/配额/审计
//! - `server`：把 Config/UserManager/FileLogger 组装为 libunftp Server 并管理生命周期

pub mod auth;
pub mod server;
pub mod storage;

pub use server::FtpServer;
