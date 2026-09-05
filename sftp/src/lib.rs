//! WFTPD SFTP 服务器
//!
//! 基于 russh（SSH 传输层）+ russh-sftp（SFTP 协议层）：
//! - `handler` 负责 SSH 认证与会话/通道管理
//! - `ops` 实现 SFTP 协议操作（路径安全、权限、配额、限速、审计）

mod handler;
mod ops;
mod server;

pub use handler::SftpHandler;
pub use ops::SftpFileHandler;
pub use server::SftpServer;
