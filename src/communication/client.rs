use anyhow::Result;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use super::protocol::*;

static REQUEST_ID: AtomicU64 = AtomicU64::new(1);

fn next_request_id() -> u64 {
    REQUEST_ID.fetch_add(1, Ordering::SeqCst)
}

async fn send_request(command: IpcCommand) -> Result<IpcResponse> {
    let socket_path = Path::new(SOCKET_PATH);
    
    if !socket_path.exists() {
        return Err(anyhow::anyhow!("IPC socket not found: {}. Is wftpd service running?", SOCKET_PATH));
    }
    
    let stream = UnixStream::connect(socket_path).await?;
    let (reader, mut writer) = stream.into_split();
    
    let request = IpcRequest::new(next_request_id(), command);
    let request_json = serde_json::to_string(&request)?;
    writer.write_all(format!("{}\n", request_json).as_bytes()).await?;
    
    let mut reader = BufReader::new(reader);
    let mut response_line = String::new();
    reader.read_line(&mut response_line).await?;
    
    let response: IpcResponse = serde_json::from_str(response_line.trim())?;
    Ok(response)
}

pub struct IpcClient;

impl IpcClient {
    pub fn get_status() -> Result<ServerStatus> {
        with_runtime(async {
            let response = send_request(IpcCommand::GetStatus).await?;
            match response.result {
                IpcResult::Status { ftp_running, sftp_running } => Ok(ServerStatus {
                    ftp_running,
                    sftp_running,
                }),
                IpcResult::Error { message } => Err(anyhow::anyhow!("{}", message)),
                _ => Err(anyhow::anyhow!("Unexpected response type")),
            }
        })
    }

    pub fn start_ftp() -> Result<IpcResponseWrapper> {
        with_runtime(async {
            let response = send_request(IpcCommand::StartFtp).await?;
            Ok(IpcResponseWrapper::from(response))
        })
    }

    pub fn stop_ftp() -> Result<IpcResponseWrapper> {
        with_runtime(async {
            let response = send_request(IpcCommand::StopFtp).await?;
            Ok(IpcResponseWrapper::from(response))
        })
    }

    pub fn start_sftp() -> Result<IpcResponseWrapper> {
        with_runtime(async {
            let response = send_request(IpcCommand::StartSftp).await?;
            Ok(IpcResponseWrapper::from(response))
        })
    }

    pub fn stop_sftp() -> Result<IpcResponseWrapper> {
        with_runtime(async {
            let response = send_request(IpcCommand::StopSftp).await?;
            Ok(IpcResponseWrapper::from(response))
        })
    }

    pub fn restart_service() -> Result<IpcResponseWrapper> {
        with_runtime(async {
            let response = send_request(IpcCommand::RestartService).await?;
            Ok(IpcResponseWrapper::from(response))
        })
    }

    pub fn reload_config() -> Result<()> {
        with_runtime(async {
            let response = send_request(IpcCommand::ReloadConfig).await?;
            match response.result {
                IpcResult::Success { .. } => Ok(()),
                IpcResult::Error { message } => Err(anyhow::anyhow!("{}", message)),
                _ => Err(anyhow::anyhow!("Unexpected response type")),
            }
        })
    }

    pub fn get_logs(count: usize) -> Result<Vec<LogEntryJson>> {
        with_runtime(async {
            let response = send_request(IpcCommand::GetLogs { count }).await?;
            match response.result {
                IpcResult::Logs { entries } => Ok(entries),
                IpcResult::Error { message } => Err(anyhow::anyhow!("{}", message)),
                _ => Err(anyhow::anyhow!("Unexpected response type")),
            }
        })
    }

    pub fn config_exists() -> Result<bool> {
        with_runtime(async {
            let response = send_request(IpcCommand::ConfigExists).await?;
            match response.result {
                IpcResult::Bool { value } => Ok(value),
                IpcResult::Error { message } => Err(anyhow::anyhow!("{}", message)),
                _ => Err(anyhow::anyhow!("Unexpected response type")),
            }
        })
    }

    pub fn users_exists() -> Result<bool> {
        with_runtime(async {
            let response = send_request(IpcCommand::UsersExists).await?;
            match response.result {
                IpcResult::Bool { value } => Ok(value),
                IpcResult::Error { message } => Err(anyhow::anyhow!("{}", message)),
                _ => Err(anyhow::anyhow!("Unexpected response type")),
            }
        })
    }
}

#[derive(Debug, Clone)]
pub struct ServerStatus {
    pub ftp_running: bool,
    pub sftp_running: bool,
}

#[derive(Debug, Clone)]
pub struct IpcResponseWrapper {
    pub success: bool,
    pub message: String,
}

impl From<IpcResponse> for IpcResponseWrapper {
    fn from(response: IpcResponse) -> Self {
        match response.result {
            IpcResult::Success { message } => Self {
                success: true,
                message,
            },
            IpcResult::Error { message } => Self {
                success: false,
                message,
            },
            _ => Self {
                success: false,
                message: "Unexpected response type".to_string(),
            },
        }
    }
}

pub fn with_runtime<F, T>(f: F) -> Result<T>
where
    F: std::future::Future<Output = Result<T>>,
{
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(f)
}

pub fn read_config() -> Result<String> {
    with_runtime(async {
        let response = send_request(IpcCommand::GetConfig).await?;
        match response.result {
            IpcResult::Config { content } => Ok(content),
            IpcResult::Error { message } => Err(anyhow::anyhow!("{}", message)),
            _ => Err(anyhow::anyhow!("Unexpected response type")),
        }
    })
}

pub fn write_config(content: &str) -> Result<()> {
    let content = content.to_string();
    with_runtime(async move {
        let response = send_request(IpcCommand::SaveConfig { content }).await?;
        match response.result {
            IpcResult::Success { .. } => Ok(()),
            IpcResult::Error { message } => Err(anyhow::anyhow!("{}", message)),
            _ => Err(anyhow::anyhow!("Unexpected response type")),
        }
    })
}

pub fn read_users() -> Result<String> {
    with_runtime(async {
        let response = send_request(IpcCommand::GetUsers).await?;
        match response.result {
            IpcResult::Users { content } => Ok(content),
            IpcResult::Error { message } => Err(anyhow::anyhow!("{}", message)),
            _ => Err(anyhow::anyhow!("Unexpected response type")),
        }
    })
}

pub fn write_users(content: &str) -> Result<()> {
    let content = content.to_string();
    with_runtime(async move {
        let response = send_request(IpcCommand::SaveUsers { content }).await?;
        match response.result {
            IpcResult::Success { .. } => Ok(()),
            IpcResult::Error { message } => Err(anyhow::anyhow!("{}", message)),
            _ => Err(anyhow::anyhow!("Unexpected response type")),
        }
    })
}

pub fn write_audit_log(user: &str, action: &str, target: &str, details: &str) -> Result<()> {
    let user = user.to_string();
    let action = action.to_string();
    let target = target.to_string();
    let details = details.to_string();
    with_runtime(async move {
        let response = send_request(IpcCommand::WriteAuditLog {
            user,
            action,
            target,
            details,
        }).await?;
        match response.result {
            IpcResult::Success { .. } => Ok(()),
            IpcResult::Error { message } => Err(anyhow::anyhow!("{}", message)),
            _ => Err(anyhow::anyhow!("Unexpected response type")),
        }
    })
}
