use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write, BufReader, BufWriter};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::io::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;

pub const SOCKET_PATH: &str = "/run/wftpd/wftpd.sock";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Command {
    pub action: String,
    pub service: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub success: bool,
    pub message: String,
    pub ftp_running: bool,
    pub sftp_running: bool,
}

impl Response {
    pub fn ok(ftp_running: bool, sftp_running: bool) -> Self {
        Response {
            success: true,
            message: "OK".to_string(),
            ftp_running,
            sftp_running,
        }
    }

    pub fn error(msg: &str) -> Self {
        Response {
            success: false,
            message: msg.to_string(),
            ftp_running: false,
            sftp_running: false,
        }
    }
}

fn get_peer_cred(stream: &UnixStream) -> Result<(u32, u32)> {
    use libc::{getsockopt, socklen_t, SOL_SOCKET, SO_PEERCRED};
    
    #[repr(C)]
    struct Ucred {
        pid: i32,
        uid: u32,
        gid: u32,
    }
    
    let fd = stream.as_raw_fd();
    let mut cred: Ucred = unsafe { std::mem::zeroed() };
    let mut len: socklen_t = std::mem::size_of::<Ucred>() as socklen_t;
    
    let result = unsafe {
        getsockopt(fd, SOL_SOCKET, SO_PEERCRED, &mut cred as *mut _ as *mut _, &mut len)
    };
    
    if result == 0 {
        Ok((cred.uid, cred.gid))
    } else {
        anyhow::bail!("Failed to get peer credentials")
    }
}

fn is_authorized(uid: u32, gid: u32) -> bool {
    if uid == 0 {
        return true;
    }
    
    let current_uid = unsafe { libc::getuid() };
    if uid == current_uid {
        return true;
    }
    
    unsafe {
        let wftpg_group_name = std::ffi::CString::new("wftpg").unwrap();
        let wftpg_group = libc::getgrnam(wftpg_group_name.as_ptr());
        if !wftpg_group.is_null() {
            let wftpg_gid = (*wftpg_group).gr_gid;
            if gid == wftpg_gid {
                return true;
            }
        }
    }
    
    false
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
    listener: UnixListener,
}

impl IpcServer {
    pub fn new() -> Result<Self> {
        let socket_path = Path::new(SOCKET_PATH);
        
        if socket_path.exists() {
            std::fs::remove_file(socket_path)?;
        }
        
        if let Some(parent) = socket_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        
        let listener = UnixListener::bind(socket_path)?;
        
        let gid_result = unsafe {
            let wftpg_group_name = std::ffi::CString::new("wftpg").unwrap();
            let wftpg_group = libc::getgrnam(wftpg_group_name.as_ptr());
            if !wftpg_group.is_null() {
                let gid = (*wftpg_group).gr_gid;
                let c_path = std::ffi::CString::new(SOCKET_PATH).unwrap();
                let result = libc::chown(c_path.as_ptr(), -1i32 as libc::uid_t, gid);
                if result == 0 {
                    log::info!("Set socket group to wftpg (gid={})", gid);
                    Ok(())
                } else {
                    log::warn!("Failed to chown socket: errno={}", std::io::Error::last_os_error());
                    Err(())
                }
            } else {
                log::warn!("wftpg group not found");
                Err(())
            }
        };
        
        if gid_result.is_err() {
            log::warn!("Falling back to world-readable socket permissions");
            std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o666))?;
        } else {
            std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o660))?;
        }
        
        Ok(IpcServer { listener })
    }
    
    pub fn accept(&self) -> Result<(UnixStream, Command)> {
        let (stream, _) = self.listener.accept()?;
        stream.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(std::time::Duration::from_secs(5)))?;
        
        let (uid, gid) = match get_peer_cred(&stream) {
            Ok(cred) => cred,
            Err(e) => {
                log::warn!("Failed to get peer credentials: {}", e);
                let _ = Self::send_error(&stream, "认证失败");
                anyhow::bail!("Authentication failed");
            }
        };
        
        if !is_authorized(uid, gid) {
            log::warn!("Unauthorized connection from uid:{} gid:{}", uid, gid);
            let _ = Self::send_error(&stream, "权限不足");
            anyhow::bail!("Unauthorized");
        }
        
        let mut reader = BufReader::new(&stream);
        let buffer = read_message(&mut reader)?;
        
        let command: Command = serde_json::from_slice(&buffer)?;
        
        Ok((stream, command))
    }
    
    fn send_error(stream: &UnixStream, msg: &str) -> Result<()> {
        let response = Response::error(msg);
        Self::send_response(stream, &response)
    }
    
    pub fn send_response(stream: &UnixStream, response: &Response) -> Result<()> {
        let json = serde_json::to_vec(response)?;
        let mut writer = BufWriter::new(stream);
        write_message(&mut writer, &json)
    }
}

pub struct IpcClient;

impl IpcClient {
    fn send_command_internal(cmd: Command, socket_path: &Path) -> Result<Response> {
        let stream = UnixStream::connect(socket_path)?;
        stream.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(std::time::Duration::from_secs(5)))?;
        
        let mut writer = BufWriter::new(&stream);
        let json = serde_json::to_vec(&cmd)?;
        write_message(&mut writer, &json)?;
        
        let mut reader = BufReader::new(&stream);
        let buffer = read_message(&mut reader)?;
        
        let response: Response = serde_json::from_slice(&buffer)?;
        Ok(response)
    }
    
    pub fn send_command(cmd: Command) -> Result<Response> {
        let socket_path = Path::new(SOCKET_PATH);
        
        if !socket_path.exists() {
            return Ok(Response::error("服务未运行"));
        }
        
        Self::send_command_internal(cmd, socket_path)
    }
    
    pub fn get_status() -> Result<Response> {
        Self::send_command(Command {
            action: "status".to_string(),
            service: None,
        })
    }
    
    pub fn start_ftp() -> Result<Response> {
        Self::send_command(Command {
            action: "start".to_string(),
            service: Some("ftp".to_string()),
        })
    }
    
    pub fn stop_ftp() -> Result<Response> {
        Self::send_command(Command {
            action: "stop".to_string(),
            service: Some("ftp".to_string()),
        })
    }
    
    pub fn start_sftp() -> Result<Response> {
        Self::send_command(Command {
            action: "start".to_string(),
            service: Some("sftp".to_string()),
        })
    }
    
    pub fn stop_sftp() -> Result<Response> {
        Self::send_command(Command {
            action: "stop".to_string(),
            service: Some("sftp".to_string()),
        })
    }
    
    pub fn start_all() -> Result<Response> {
        Self::send_command(Command {
            action: "start".to_string(),
            service: Some("all".to_string()),
        })
    }
    
    pub fn stop_all() -> Result<Response> {
        Self::send_command(Command {
            action: "stop".to_string(),
            service: Some("all".to_string()),
        })
    }
}
