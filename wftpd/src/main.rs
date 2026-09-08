//! wftpd 后端守护进程
//!
//! 职责：
//! 1. 按配置组装并运行 FTP / SFTP 服务（单进程内独立任务，可分别启停）
//! 2. 在 UDS 上提供 gRPC 控制服务（wftpd.v1.Control），供 wftp-gui 前端使用
//!
//! 运行模型：以当前桌面登录用户运行（systemd --user 服务），所有可写路径
//! 遵循 XDG 规范（见 `wftpd_common::paths`）。

// 本 crate 为应用型项目内部代码（不作为库对外发布）：pedantic 的文档规范类
// lint（# Errors/# Panics 章节、#[must_use] 标注）与函数长度上限对内部 API
// 收益有限，统一在 crate 级关闭；具体取舍见仓库审计说明。
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::too_many_lines
)]
mod control;
mod logs;
mod state;

use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;

use tokio::net::UnixListener;
use tokio_stream::wrappers::UnixListenerStream;
use tracing::{error, info};

use state::BackendState;

fn main() -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    runtime.block_on(run())
}

async fn run() -> anyhow::Result<()> {
    let state = Arc::new(BackendState::new()?);

    info!("wftpd v{} starting", env!("CARGO_PKG_VERSION"));

    // 根据配置的 enabled 标志启动 FTP / SFTP
    state.apply_enabled_flags().await;

    // gRPC 控制服务（UDS）
    let socket_path = wftpd_common::paths::socket_path();
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)?;
        let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
    }
    // 清理上次运行残留的套接字文件
    let _ = std::fs::remove_file(&socket_path);

    let uds = UnixListener::bind(&socket_path)?;
    std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))?;
    info!("control service listening on {}", socket_path.display());

    let result = tonic::transport::Server::builder()
        .add_service(wftpd_proto::ControlServer::new(
            control::ControlService::new(Arc::clone(&state)),
        ))
        .serve_with_incoming_shutdown(UnixListenerStream::new(uds), shutdown_signal())
        .await;

    if let Err(e) = result {
        error!("control service error: {}", e);
    }

    state.stop_all().await;
    let _ = std::fs::remove_file(&socket_path);
    info!("wftpd shutdown complete");

    Ok(())
}

async fn shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};

    let mut sigterm = signal(SignalKind::terminate()).expect("install SIGTERM handler");
    let mut sigint = signal(SignalKind::interrupt()).expect("install SIGINT handler");

    tokio::select! {
        _ = sigterm.recv() => info!("received SIGTERM, shutting down"),
        _ = sigint.recv() => info!("received SIGINT, shutting down"),
    }
}
