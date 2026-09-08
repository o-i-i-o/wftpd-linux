//! gRPC(tonic over UDS) 客户端封装。
//!
//! 函数签名与旧 IPC 客户端保持一致：阻塞式、返回 `anyhow::Result`，
//! UI 侧在独立线程中调用（见 `log_tab` / `user_tab` 等的用法）。

use std::sync::OnceLock;

use anyhow::{Result, anyhow};
use hyper_util::rt::TokioIo;
use tonic::transport::{Channel, Endpoint};
use tower::service_fn;
use wftpd_common::paths;
use wftpd_common::{FileLogEntryJson, LogEntryJson};
use wftpd_proto::wftpd::v1::service_selector::Which;
use wftpd_proto::{
    ControlClient, GetFileOpLogContentRequest, GetLogFileContentRequest, GetRecentLogsRequest,
    GetStatusRequest, SaveConfigRequest, SaveUsersRequest, ServiceSelector, WriteAuditLogRequest,
};

/// 进程级共享的 tokio Runtime（GTK 主循环不参与异步调度）
fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("failed to build tokio runtime")
    })
}

async fn connect() -> Result<ControlClient<Channel>> {
    let socket_path = paths::socket_path();
    if !socket_path.exists() {
        return Err(anyhow!(connect_error_hint()));
    }

    let path = socket_path.clone();
    let channel = Endpoint::from_static("http://unix")
        .connect_with_connector(service_fn(move |_: tonic::transport::Uri| {
            let path = path.clone();
            async move {
                let stream = tokio::net::UnixStream::connect(path).await?;
                Ok::<_, std::io::Error>(TokioIo::new(stream))
            }
        }))
        .await
        .map_err(|e| anyhow!("连接 wftpd 失败 ({}): {e}", socket_path.display()))?;

    Ok(ControlClient::new(channel))
}

#[must_use]
pub fn connect_error_hint() -> String {
    format!(
        "未找到 wftpd 控制套接字 ({}),后端服务可能未运行。可执行: systemctl --user start wftpd",
        paths::socket_path().display()
    )
}

fn run<F, T>(f: F) -> Result<T>
where
    F: std::future::Future<Output = Result<T>>,
{
    runtime().block_on(f)
}

// ===== 服务状态与生命周期 =====

#[derive(Debug, Clone)]
pub struct ServerStatus {
    pub ftp_running: bool,
    pub sftp_running: bool,
    pub version: String,
}

/// 查询后端版本与 FTP/SFTP 运行状态
///
/// # Errors
/// wftpd 控制套接字不存在（后端未运行）、UDS 连接失败，或 gRPC 调用失败时返回错误
pub fn get_status() -> Result<ServerStatus> {
    run(async {
        let mut client = connect().await?;
        let response = client.get_status(GetStatusRequest {}).await?;
        let status = response.into_inner();
        Ok(ServerStatus {
            ftp_running: status.ftp_running,
            sftp_running: status.sftp_running,
            version: status.version,
        })
    })
}

fn selector(which: Which) -> ServiceSelector {
    ServiceSelector {
        which: which as i32,
    }
}

fn op_result(reply: &wftpd_proto::OpReply) -> Result<()> {
    if reply.success {
        Ok(())
    } else {
        Err(anyhow!("{}", reply.message))
    }
}

/// 启动指定服务；业务失败经 `OpReply` 返回，不产生 `Err`
///
/// # Errors
/// wftpd 控制套接字不存在（后端未运行）、UDS 连接失败，或 gRPC 调用失败时返回错误
pub fn start_service(which: Which) -> Result<()> {
    run(async {
        let mut client = connect().await?;
        let response = client.start_service(selector(which)).await?;
        op_result(&response.into_inner())
    })
}

/// 停止指定服务；业务失败经 `OpReply` 返回，不产生 `Err`
///
/// # Errors
/// wftpd 控制套接字不存在（后端未运行）、UDS 连接失败，或 gRPC 调用失败时返回错误
pub fn stop_service(which: Which) -> Result<()> {
    run(async {
        let mut client = connect().await?;
        let response = client.stop_service(selector(which)).await?;
        op_result(&response.into_inner())
    })
}

/// 重启指定服务；业务失败经 `OpReply` 返回，不产生 `Err`
///
/// # Errors
/// wftpd 控制套接字不存在（后端未运行）、UDS 连接失败，或 gRPC 调用失败时返回错误
pub fn restart_service(which: Which) -> Result<()> {
    run(async {
        let mut client = connect().await?;
        let response = client.restart_service(selector(which)).await?;
        op_result(&response.into_inner())
    })
}

// ===== 配置与用户 =====

/// 保存配置（后端负责校验与落盘），返回规范化后的配置内容
/// 保存配置（后端负责校验与落盘），返回规范化后的配置内容
///
/// # Errors
/// wftpd 控制套接字不存在（后端未运行）、UDS 连接失败，或 gRPC 调用失败时返回错误
pub fn write_config(content: &str) -> Result<String> {
    let content = content.to_string();
    run(async {
        let mut client = connect().await?;
        let response = client
            .save_config(SaveConfigRequest { content })
            .await
            .map_err(|e| anyhow!("保存配置失败: {e}"))?;
        Ok(response.into_inner().content)
    })
}

/// 保存用户库（后端负责校验与落盘），返回实际保存的内容
/// 保存用户库（后端负责校验与落盘），返回实际保存的内容
///
/// # Errors
/// wftpd 控制套接字不存在（后端未运行）、UDS 连接失败，或 gRPC 调用失败时返回错误
pub fn write_users(content: &str) -> Result<String> {
    let content = content.to_string();
    run(async {
        let mut client = connect().await?;
        let response = client
            .save_users(SaveUsersRequest { content })
            .await
            .map_err(|e| anyhow!("保存用户失败: {e}"))?;
        Ok(response.into_inner().content)
    })
}

/// 追加一条 GUI 审计记录到后端审计日志
///
/// # Errors
/// wftpd 控制套接字不存在（后端未运行）、UDS 连接失败，或 gRPC 调用失败时返回错误
pub fn write_audit_log(user: &str, action: &str, target: &str, details: &str) -> Result<()> {
    let request = WriteAuditLogRequest {
        user: user.to_string(),
        action: action.to_string(),
        target: target.to_string(),
        details: details.to_string(),
    };
    run(async {
        let mut client = connect().await?;
        let response = client.write_audit_log(request).await?;
        op_result(&response.into_inner())
    })
}

// ===== 日志 =====

/// 读取后端内存环形缓冲中的最近日志
/// 读取后端内存环形缓冲中的最近日志
///
/// # Errors
/// wftpd 控制套接字不存在（后端未运行）、UDS 连接失败，或 gRPC 调用失败时返回错误
pub fn get_logs(count: usize) -> Result<Vec<LogEntryJson>> {
    run(async {
        let mut client = connect().await?;
        let response = client
            .get_recent_logs(GetRecentLogsRequest {
                count: u32::try_from(count).unwrap_or(u32::MAX),
            })
            .await
            .map_err(|e| anyhow!("读取日志失败: {e}"))?;
        Ok(response
            .into_inner()
            .entries
            .into_iter()
            .map(Into::into)
            .collect())
    })
}

/// 读取指定程序日志文件的最后 count 条
/// 读取指定程序日志文件的最后 count 条
///
/// # Errors
/// wftpd 控制套接字不存在（后端未运行）、UDS 连接失败，或 gRPC 调用失败时返回错误
pub fn get_log_file_content(path: &str, count: usize) -> Result<Vec<LogEntryJson>> {
    let path = path.to_string();
    run(async {
        let mut client = connect().await?;
        let response = client
            .get_log_file_content(GetLogFileContentRequest {
                path,
                count: u32::try_from(count).unwrap_or(u32::MAX),
            })
            .await
            .map_err(|e| anyhow!("读取日志文件失败: {e}"))?;
        Ok(response
            .into_inner()
            .entries
            .into_iter()
            .map(Into::into)
            .collect())
    })
}

/// 读取文件操作审计日志；path == "current" 表示内存缓冲中的最新记录
/// 读取文件操作审计日志；`path == "current"` 表示内存缓冲中的最新记录
///
/// # Errors
/// wftpd 控制套接字不存在（后端未运行）、UDS 连接失败，或 gRPC 调用失败时返回错误
pub fn get_file_log_file_content(path: &str, count: usize) -> Result<Vec<FileLogEntryJson>> {
    let path = path.to_string();
    run(async {
        let mut client = connect().await?;
        let response = client
            .get_file_op_log_content(GetFileOpLogContentRequest {
                path,
                count: u32::try_from(count).unwrap_or(u32::MAX),
            })
            .await
            .map_err(|e| anyhow!("读取文件操作日志失败: {e}"))?;
        Ok(response
            .into_inner()
            .entries
            .into_iter()
            .map(Into::into)
            .collect())
    })
}
