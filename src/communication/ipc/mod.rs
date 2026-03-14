use anyhow::Result;

pub struct IpcClient;
pub struct Command;
pub struct Response;

#[derive(Debug, Clone)]
pub struct IpcResponse {
    pub success: bool,
    pub message: String,
}

impl IpcClient {
    pub fn get_status() -> Result<ServerStatus> {
        Ok(ServerStatus {
            ftp_running: false,
            sftp_running: false,
        })
    }
    
    pub fn start_ftp() -> Result<IpcResponse> {
        Ok(IpcResponse {
            success: true,
            message: "FTP服务器启动成功".to_string(),
        })
    }
    
    pub fn stop_ftp() -> Result<IpcResponse> {
        Ok(IpcResponse {
            success: true,
            message: "FTP服务器停止成功".to_string(),
        })
    }
    
    pub fn start_sftp() -> Result<IpcResponse> {
        Ok(IpcResponse {
            success: true,
            message: "SFTP服务器启动成功".to_string(),
        })
    }
    
    pub fn stop_sftp() -> Result<IpcResponse> {
        Ok(IpcResponse {
            success: true,
            message: "SFTP服务器停止成功".to_string(),
        })
    }
}

pub struct ServerStatus {
    pub ftp_running: bool,
    pub sftp_running: bool,
}
