//! WFTPD SFTP 服务器
//!
//! 基于 russh（SSH 传输层）+ russh-sftp（SFTP 协议层）：
//! - `handler` 负责 SSH 认证与会话/通道管理
//! - `ops` 实现 SFTP 协议操作（路径安全、权限、配额、限速、审计）

// 本 crate 为应用型项目内部代码（不作为库对外发布）：pedantic 的文档规范类
// lint（# Errors/# Panics 章节、#[must_use] 标注）与函数长度上限对内部 API
// 收益有限，统一在 crate 级关闭；具体取舍见仓库审计说明。
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::too_many_lines
)]
// russh / russh-sftp 的 trait 方法声明为 async，实现体无 await 点时
// clippy 会提示 unused_async_trait_impl，但签名不可更改。
#![allow(clippy::unused_async_trait_impl)]
mod handler;
mod ops;
mod server;

pub use handler::SftpHandler;
pub use ops::SftpFileHandler;
pub use server::SftpServer;
