use tokio::signal;
use tracing::{info, error};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 初始化应用状态（包含日志系统）
    let mut app_state = wftpd::AppState::new()?;
    
    // 根据配置启动 FTP 和/或 SFTP 服务
    let (ftp_enabled, sftp_enabled) = {
        let cfg = app_state.config.lock().unwrap();
        (cfg.ftp.enabled, cfg.sftp.enabled)
    };
    
    if ftp_enabled {
        if let Err(e) = app_state.start_ftp().await {
            error!("Failed to start FTP server: {}", e);
        } else {
            info!("FTP server started successfully");
        }
    }
    
    if sftp_enabled {
        if let Err(e) = app_state.start_sftp().await {
            error!("Failed to start SFTP server: {}", e);
        } else {
            info!("SFTP server started successfully");
        }
    }
    
    info!("WFTPD service started successfully");
    
    // 等待退出信号
    match signal::ctrl_c().await {
        Ok(()) => {
            info!("Received shutdown signal");
        }
        Err(e) => {
            error!("Failed to listen for shutdown signal: {}", e);
        }
    }
    
    // 停止所有服务
    info!("WFTPD service shutting down");
    app_state.stop_all().await;
    
    Ok(())
}
