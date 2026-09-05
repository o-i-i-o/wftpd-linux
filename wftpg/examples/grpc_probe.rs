//! gRPC 控制面探测工具：连接运行中的 wftpd，验证状态/日志/配置读写等接口。
//!
//! 用法：cargo run -p wftpg --example grpc_probe

use hyper_util::rt::TokioIo;
use tonic::transport::{Channel, Endpoint};
use tower::service_fn;
use wftpd_proto::wftpd::v1::service_selector::Which;
use wftpd_proto::{GetRecentLogsRequest, GetStatusRequest, ServiceSelector, WatchLogsRequest};

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let socket_path = wftpd_common::paths::socket_path();
    if !socket_path.exists() {
        anyhow::bail!("socket not found: {}", socket_path.display());
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
        .await?;

    let mut client = wftpd_proto::ControlClient::new(channel);

    // 1. GetStatus
    let status = client.get_status(GetStatusRequest {}).await?.into_inner();
    println!(
        "GetStatus: ftp={} sftp={} version={}",
        status.ftp_running, status.sftp_running, status.version
    );

    // 2. GetRecentLogs
    let logs = client
        .get_recent_logs(GetRecentLogsRequest { count: 5 })
        .await?
        .into_inner();
    println!(
        "GetRecentLogs: {} entries; first: {:?}",
        logs.entries.len(),
        logs.entries
            .first()
            .map(|e| (e.level.clone(), e.message.clone()))
    );

    // 3. WatchLogs（流式，收 2 条后退出）
    let mut stream = client
        .watch_logs(WatchLogsRequest { tail_count: 2 })
        .await?
        .into_inner();
    let mut received = 0;
    while let Some(event) = stream.message().await? {
        let entry = event.entry.unwrap_or_default();
        println!("WatchLogs: [{}] {}", entry.level, entry.message);
        received += 1;
        if received >= 2 {
            break;
        }
    }

    // 4. RestartService(FTP / SFTP)
    let reply = client
        .restart_service(ServiceSelector {
            which: Which::Ftp as i32,
        })
        .await?
        .into_inner();
    println!(
        "RestartService(FTP): success={} msg={}",
        reply.success, reply.message
    );

    let reply = client
        .restart_service(ServiceSelector {
            which: Which::Sftp as i32,
        })
        .await?
        .into_inner();
    println!(
        "RestartService(SFTP): success={} msg={}",
        reply.success, reply.message
    );

    // 5. 配置读→写往返
    let config = client
        .get_config(wftpd_proto::GetConfigRequest {})
        .await?
        .into_inner()
        .content;
    let saved = client
        .save_config(wftpd_proto::SaveConfigRequest { content: config })
        .await?
        .into_inner()
        .content;
    println!("SaveConfig roundtrip: {} bytes", saved.len());

    // 6. 用户读→写往返
    let users = client
        .get_users(wftpd_proto::GetUsersRequest {})
        .await?
        .into_inner()
        .content;
    let saved_users = client
        .save_users(wftpd_proto::SaveUsersRequest { content: users })
        .await?
        .into_inner()
        .content;
    println!("SaveUsers roundtrip: {} bytes", saved_users.len());

    // 7. 再次确认状态
    let status = client.get_status(GetStatusRequest {}).await?.into_inner();
    println!(
        "GetStatus after restarts: ftp={} sftp={}",
        status.ftp_running, status.sftp_running
    );

    println!("ALL PROBES OK");
    Ok(())
}
