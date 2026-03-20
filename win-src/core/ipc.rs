use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write, BufReader, BufWriter};
use std::time::Duration;

use crate::core::windows_ipc::{IpcServerInner, IpcStream};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Command {
    pub action: String,
    pub service: Option<String>,
    pub data: Option<CommandData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum CommandData {
    GetLogs { count: usize, log_type: String },
    GetFileLogs { count: usize },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub success: bool,
    pub message: String,
    pub ftp_running: bool,
    pub sftp_running: bool,
    pub logs: Option<Vec<LogEntryDto>>,
    pub file_logs: Option<Vec<FileLogEntryDto>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntryDto {
    pub timestamp: String,
    pub level: String,
    pub source: String,
    pub message: String,
    pub client_ip: Option<String>,
    pub username: Option<String>,
    pub action: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileLogEntryDto {
    pub timestamp: String,
    pub username: String,
    pub client_ip: String,
    pub operation: String,
    pub file_path: String,
    pub file_size: u64,
    pub protocol: String,
    pub success: bool,
    pub message: String,
}

impl Response {
    pub fn ok(ftp_running: bool, sftp_running: bool) -> Self {
        Response {
            success: true,
            message: "OK".to_string(),
            ftp_running,
            sftp_running,
            logs: None,
            file_logs: None,
        }
    }

    pub fn error(msg: &str) -> Self {
        Response {
            success: false,
            message: msg.to_string(),
            ftp_running: false,
            sftp_running: false,
            logs: None,
            file_logs: None,
        }
    }

    pub fn with_logs(ftp_running: bool, sftp_running: bool, logs: Vec<LogEntryDto>) -> Self {
        Response {
            success: true,
            message: "OK".to_string(),
            ftp_running,
            sftp_running,
            logs: Some(logs),
            file_logs: None,
        }
    }

    pub fn with_file_logs(ftp_running: bool, sftp_running: bool, file_logs: Vec<FileLogEntryDto>) -> Self {
        Response {
            success: true,
            message: "OK".to_string(),
            ftp_running,
            sftp_running,
            logs: None,
            file_logs: Some(file_logs),
        }
    }
}

fn read_message<R: Read>(reader: &mut R) -> Result<Vec<u8>> {
    let mut len_bytes = [0u8; 4];
    reader.read_exact(&mut len_bytes)?;
    let len = u32::from_be_bytes(len_bytes) as usize;
    
    let mut buffer = vec![0u8; len];
    reader.read_exact(&mut buffer)?;
    
    Ok(buffer)
}

fn write_message<W: Write>(writer: &mut W, data: &[u8]) -> Result<()> {
    let len = data.len() as u32;
    writer.write_all(&len.to_be_bytes())?;
    writer.write_all(data)?;
    writer.flush()?;
    Ok(())
}


pub struct IpcServer {
    inner: IpcServerInner,
}

impl IpcServer {
    pub fn new() -> Result<Self> {
        Ok(IpcServer {
            inner: IpcServerInner::new()?,
        })
    }
    
    pub fn accept(&self) -> Result<(IpcStream, Command)> {
        let stream = self.inner.accept()?;
        
        let mut reader = BufReader::new(&stream);
        let buffer = read_message(&mut reader)?;
        
        let command: Command = serde_json::from_slice(&buffer)?;
        
        Ok((stream, command))
    }
    
    pub fn accept_timeout(&self, timeout: Duration) -> Result<Option<(IpcStream, Command)>> {
        match self.inner.accept_timeout(timeout)? {
            Some(stream) => {
                let mut reader = BufReader::new(&stream);
                let buffer = read_message(&mut reader)?;
                
                let command: Command = serde_json::from_slice(&buffer)?;
                
                Ok(Some((stream, command)))
            }
            None => Ok(None),
        }
    }
    
    pub fn send_response(stream: &IpcStream, response: &Response) -> Result<()> {
        let json = serde_json::to_vec(response)?;
        let mut writer = BufWriter::new(stream);
        write_message(&mut writer, &json)
    }
}

pub struct IpcClient;

impl IpcClient {
    fn send_command_internal(cmd: Command) -> Result<Response> {
        let stream = IpcStream::connect()?;
        
        let mut writer = BufWriter::new(&stream);
        let json = serde_json::to_vec(&cmd)?;
        write_message(&mut writer, &json)?;
        
        let mut reader = BufReader::new(&stream);
        let buffer = read_message(&mut reader)?;
        
        let response: Response = serde_json::from_slice(&buffer)?;
        Ok(response)
    }
    
    pub fn send_command(cmd: Command) -> Result<Response> {
        Self::send_command_internal(cmd)
    }
    
    pub fn get_status() -> Result<Response> {
        Self::send_command(Command {
            action: "status".to_string(),
            service: None,
            data: None,
        })
    }
    
    pub fn start_ftp() -> Result<Response> {
        Self::send_command(Command {
            action: "start".to_string(),
            service: Some("ftp".to_string()),
            data: None,
        })
    }
    
    pub fn stop_ftp() -> Result<Response> {
        Self::send_command(Command {
            action: "stop".to_string(),
            service: Some("ftp".to_string()),
            data: None,
        })
    }
    
    pub fn start_sftp() -> Result<Response> {
        Self::send_command(Command {
            action: "start".to_string(),
            service: Some("sftp".to_string()),
            data: None,
        })
    }
    
    pub fn stop_sftp() -> Result<Response> {
        Self::send_command(Command {
            action: "stop".to_string(),
            service: Some("sftp".to_string()),
            data: None,
        })
    }
    
    pub fn start_all() -> Result<Response> {
        Self::send_command(Command {
            action: "start".to_string(),
            service: Some("all".to_string()),
            data: None,
        })
    }
    
    pub fn stop_all() -> Result<Response> {
        Self::send_command(Command {
            action: "stop".to_string(),
            service: Some("all".to_string()),
            data: None,
        })
    }
    
    pub fn is_server_running() -> bool {
        Self::get_status().is_ok()
    }

    pub fn get_logs(count: usize) -> Result<Response> {
        Self::send_command(Command {
            action: "get_logs".to_string(),
            service: None,
            data: Some(CommandData::GetLogs {
                count,
                log_type: "all".to_string(),
            }),
        })
    }

    pub fn get_file_logs(count: usize) -> Result<Response> {
        Self::send_command(Command {
            action: "get_file_logs".to_string(),
            service: None,
            data: Some(CommandData::GetFileLogs { count }),
        })
    }
}
