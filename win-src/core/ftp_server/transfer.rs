use anyhow::Result;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::net::TcpStream;
use std::net::ToSocketAddrs;

use super::passive::PassiveManager;
use crate::core::rate_limiter::RateLimiter;

const DATA_BUFFER_SIZE: usize = 8192;

pub async fn get_data_connection(
    passive_mode: bool,
    data_port: Option<u16>,
    data_addr: &Option<String>,
    remote_ip: &str,
    passive_manager: &mut PassiveManager,
) -> Result<TcpStream> {
    let port = match data_port {
        Some(p) => p,
        None => anyhow::bail!("No data port specified"),
    };

    log::debug!("get_data_connection: passive_mode={}, port={}, remote_ip={}", passive_mode, port, remote_ip);

    if passive_mode {
        let listener = passive_manager.get_listener(port);

        if let Some(listener) = listener {
            log::debug!("Attempting to accept passive connection on port {}", port);
            match tokio::time::timeout(
                Duration::from_secs(30),
                listener.accept()
            ).await {
                Ok(Ok((stream, addr))) => {
                    log::debug!("Passive connection accepted from {}", addr);
                    Ok(stream)
                }
                Ok(Err(e)) => {
                    log::error!("Failed to accept passive connection: {}", e);
                    anyhow::bail!("Failed to accept passive connection: {}", e);
                }
                Err(_) => {
                    log::error!("Passive connection timeout on port {}", port);
                    anyhow::bail!("Passive connection timeout");
                }
            }
        } else {
            log::error!("No passive listener found for port {}", port);
            anyhow::bail!("No passive listener");
        }
    } else if let Some(addr) = data_addr {
        log::debug!("Active mode: connecting to {}", addr);
        let socket_addr = addr.to_socket_addrs()
            .map_err(|e| anyhow::anyhow!("Invalid address {}: {}", addr, e))?
            .next()
            .ok_or_else(|| anyhow::anyhow!("Could not resolve address: {}", addr))?;
        let stream = tokio::time::timeout(
            Duration::from_secs(30),
            TcpStream::connect(&socket_addr)
        ).await
            .map_err(|_| anyhow::anyhow!("Connection timeout"))?
            .map_err(|e| anyhow::anyhow!("Failed to connect: {}", e))?;
        Ok(stream)
    } else {
        let addr = format!("{}:{}", remote_ip, port);
        log::debug!("Active mode (fallback): connecting to {}", addr);
        let socket_addr = addr.to_socket_addrs()
            .map_err(|e| anyhow::anyhow!("Invalid address {}: {}", addr, e))?
            .next()
            .ok_or_else(|| anyhow::anyhow!("Could not resolve address: {}", addr))?;
        let stream = tokio::time::timeout(
            Duration::from_secs(30),
            TcpStream::connect(&socket_addr)
        ).await
            .map_err(|_| anyhow::anyhow!("Connection timeout"))?
            .map_err(|e| anyhow::anyhow!("Failed to connect: {}", e))?;
        Ok(stream)
    }
}

pub async fn receive_file(
    data_stream: &mut TcpStream,
    file_path: &Path,
    offset: u64,
    abort: Arc<AtomicBool>,
    is_ascii: bool,
) -> Result<u64> {
    log::debug!("receive_file: path={}, offset={}, is_ascii={}", file_path.display(), offset, is_ascii);
    
    let file_result = if offset > 0 {
        tokio::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(file_path).await
    } else {
        tokio::fs::File::create(file_path).await
    };

    let mut file = match file_result {
        Ok(f) => f,
        Err(e) => {
            log::error!("Failed to create/open file '{}': {}", file_path.display(), e);
            return Err(anyhow::anyhow!("Failed to create file: {}", e));
        }
    };
    
    if offset > 0
        && let Err(e) = file.seek(std::io::SeekFrom::Start(offset)).await
    {
        log::error!("Failed to seek in file '{}': {}", file_path.display(), e);
        return Err(anyhow::anyhow!("Seek failed: {}", e));
    }

    let mut buf = [0u8; DATA_BUFFER_SIZE];
    let mut total_written: u64 = 0;
    let mut transfer_error: Option<anyhow::Error> = None;
    
    loop {
        if abort.load(Ordering::Relaxed) {
            log::debug!("Transfer aborted by client");
            break;
        }
        match data_stream.read(&mut buf).await {
            Ok(0) => {
                log::debug!("Data connection closed (EOF), total written: {}", total_written);
                break;
            }
            Ok(n) => {
                let data = if is_ascii {
                    convert_crlf_to_lf(&buf[..n])
                } else {
                    buf[..n].to_vec()
                };
                if let Err(e) = file.write_all(&data).await {
                    log::error!("STOR write error for file '{}': {}", file_path.display(), e);
                    transfer_error = Some(anyhow::anyhow!("Write error: {}", e));
                    break;
                }
                total_written += data.len() as u64;
            }
            Err(e) => {
                log::error!("STOR read error from data stream: {}", e);
                transfer_error = Some(anyhow::anyhow!("Read error: {}", e));
                break;
            }
        }
    }

    if let Err(e) = file.sync_all().await {
        log::error!("Failed to sync file {:?}: {}", file_path, e);
    }

    if let Some(e) = transfer_error {
        log::error!("STOR failed: {} bytes written before error to {}", total_written, file_path.display());
        return Err(e);
    }

    log::info!("STOR completed: {} bytes written to {}", total_written, file_path.display());
    Ok(total_written)
}

pub async fn receive_file_append(
    data_stream: &mut TcpStream,
    file_path: &Path,
    abort: Arc<AtomicBool>,
    is_ascii: bool,
) -> Result<u64> {
    let mut file = tokio::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(file_path).await?;

    let mut buf = [0u8; DATA_BUFFER_SIZE];
    let mut total_written: u64 = 0;
    let mut transfer_error: Option<anyhow::Error> = None;
    
    loop {
        if abort.load(Ordering::Relaxed) {
            break;
        }
        match data_stream.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => {
                let data = if is_ascii {
                    convert_crlf_to_lf(&buf[..n])
                } else {
                    buf[..n].to_vec()
                };
                if let Err(e) = file.write_all(&data).await {
                    log::error!("APPE write error for file '{}': {}", file_path.display(), e);
                    transfer_error = Some(anyhow::anyhow!("Write error: {}", e));
                    break;
                }
                total_written += data.len() as u64;
            }
            Err(e) => {
                log::error!("APPE read error from data stream: {}", e);
                transfer_error = Some(anyhow::anyhow!("Read error: {}", e));
                break;
            }
        }
    }

    if let Err(e) = file.sync_all().await {
        log::error!("Failed to sync file {:?}: {}", file_path, e);
    }

    if let Some(e) = transfer_error {
        return Err(e);
    }

    Ok(total_written)
}

pub async fn receive_file_with_limits(
    data_stream: &mut TcpStream,
    file_path: &Path,
    offset: u64,
    abort: Arc<AtomicBool>,
    is_ascii: bool,
    rate_limiter: Option<&RateLimiter>,
) -> Result<u64> {
    log::debug!("receive_file_with_limits: path={}, offset={}, is_ascii={}", file_path.display(), offset, is_ascii);
    
    let file_result = if offset > 0 {
        tokio::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(file_path).await
    } else {
        tokio::fs::File::create(file_path).await
    };

    let mut file = match file_result {
        Ok(f) => f,
        Err(e) => {
            log::error!("Failed to create/open file '{}': {}", file_path.display(), e);
            return Err(anyhow::anyhow!("Failed to create file: {}", e));
        }
    };
    
    if offset > 0
        && let Err(e) = file.seek(std::io::SeekFrom::Start(offset)).await
    {
        log::error!("Failed to seek in file '{}': {}", file_path.display(), e);
        return Err(anyhow::anyhow!("Seek failed: {}", e));
    }

    let mut buf = [0u8; DATA_BUFFER_SIZE];
    let mut total_written: u64 = 0;
    let mut transfer_error: Option<anyhow::Error> = None;
    
    loop {
        if abort.load(Ordering::Relaxed) {
            log::debug!("Transfer aborted by client");
            break;
        }
        match data_stream.read(&mut buf).await {
            Ok(0) => {
                log::debug!("Data connection closed (EOF), total written: {}", total_written);
                break;
            }
            Ok(n) => {
                if let Some(limiter) = rate_limiter {
                    limiter.acquire(n).await;
                }
                
                let data = if is_ascii {
                    convert_crlf_to_lf(&buf[..n])
                } else {
                    buf[..n].to_vec()
                };
                if let Err(e) = file.write_all(&data).await {
                    log::error!("STOR write error for file '{}': {}", file_path.display(), e);
                    transfer_error = Some(anyhow::anyhow!("Write error: {}", e));
                    break;
                }
                total_written += data.len() as u64;
            }
            Err(e) => {
                log::error!("STOR read error from data stream: {}", e);
                transfer_error = Some(anyhow::anyhow!("Read error: {}", e));
                break;
            }
        }
    }

    if let Err(e) = file.sync_all().await {
        log::error!("Failed to sync file {:?}: {}", file_path, e);
    }

    if let Some(e) = transfer_error {
        log::error!("STOR failed: {} bytes written before error to {}", total_written, file_path.display());
        return Err(e);
    }

    log::info!("STOR completed: {} bytes written to {}", total_written, file_path.display());
    Ok(total_written)
}

pub async fn send_file_with_limits(
    data_stream: &mut TcpStream,
    file_path: &Path,
    offset: u64,
    abort: Arc<AtomicBool>,
    is_ascii: bool,
    rate_limiter: Option<&RateLimiter>,
) -> Result<()> {
    let mut file = tokio::fs::File::open(file_path).await?;
    
    if offset > 0 {
        file.seek(std::io::SeekFrom::Start(offset)).await?;
    }

    let mut buf = [0u8; DATA_BUFFER_SIZE];
    let mut transfer_error: Option<anyhow::Error> = None;
    
    loop {
        if abort.load(Ordering::Relaxed) {
            break;
        }
        match file.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => {
                if let Some(limiter) = rate_limiter {
                    limiter.acquire(n).await;
                }
                
                let data = if is_ascii {
                    convert_lf_to_crlf(&buf[..n])
                } else {
                    buf[..n].to_vec()
                };
                if let Err(e) = data_stream.write_all(&data).await {
                    log::error!("RETR write error for file '{}': {}", file_path.display(), e);
                    transfer_error = Some(anyhow::anyhow!("Write error: {}", e));
                    break;
                }
            }
            Err(e) => {
                log::error!("RETR read error from file '{}': {}", file_path.display(), e);
                transfer_error = Some(anyhow::anyhow!("Read error: {}", e));
                break;
            }
        }
    }

    if let Some(e) = transfer_error {
        return Err(e);
    }

    Ok(())
}

pub async fn send_directory_listing(
    data_stream: &mut TcpStream,
    dir_path: &Path,
    username: &str,
    is_nlst: bool,
    _is_ascii: bool,
) -> Result<()> {
    let mut dir = tokio::fs::read_dir(dir_path).await?;
    
    while let Ok(Some(entry)) = dir.next_entry().await {
        if let Ok(metadata) = entry.metadata().await {
            let name = entry.file_name().to_string_lossy().to_string();
            
            if is_nlst {
                let line = format!("{}\r\n", name);
                let _ = data_stream.write_all(line.as_bytes()).await;
            } else {
                let is_dir = metadata.is_dir();
                let perms = if is_dir {
                    "drwxr-xr-x"
                } else {
                    "-rw-r--r--"
                };
                let size = metadata.len();
                let mtime = get_file_mtime(&metadata);
                let nlink = if is_dir { 2 } else { 1 };
                let line = format!(
                    "{} {:>2} {:<8} {:<8} {:>10} {} {}\r\n",
                    perms, nlink, username, username, size, mtime, name
                );
                let _ = data_stream.write_all(line.as_bytes()).await;
            }
        }
    }

    Ok(())
}

pub async fn send_mlsd_listing(
    data_stream: &mut TcpStream,
    dir_path: &Path,
    owner: &str,
) -> Result<()> {
    let mut dir = tokio::fs::read_dir(dir_path).await?;
    
    while let Ok(Some(entry)) = dir.next_entry().await {
        if let Ok(metadata) = entry.metadata().await {
            let name = entry.file_name().to_string_lossy().to_string();
            let facts = build_mlst_facts(&metadata, owner);
            let line = format!("{} {}\r\n", facts, name);
            let _ = data_stream.write_all(line.as_bytes()).await;
        }
    }

    Ok(())
}

pub fn get_file_mtime(metadata: &std::fs::Metadata) -> String {
    if let Ok(time) = metadata.modified() {
        let dt: chrono::DateTime<chrono::Local> = time.into();
        return dt.format("%Y-%m-%d %H:%M").to_string();
    }
    "1970-01-01 00:00".to_string()
}

pub fn get_file_mtime_raw(metadata: &std::fs::Metadata) -> String {
    use std::time::UNIX_EPOCH;
    if let Ok(time) = metadata.modified()
        && let Ok(_d) = time.duration_since(UNIX_EPOCH) {
            let dt: chrono::DateTime<chrono::Local> = time.into();
            return dt.format("%Y%m%d%H%M%S").to_string();
        }
    "19700101000000".to_string()
}

pub fn build_mlst_facts(metadata: &std::fs::Metadata, owner: &str) -> String {
    let mut facts: Vec<String> = Vec::new();

    if metadata.is_dir() {
        facts.push("type=dir".to_string());
    } else {
        facts.push("type=file".to_string());
    }

    facts.push(format!("size={}", metadata.len()));

    if let Ok(time) = metadata.modified() {
        let dt: chrono::DateTime<chrono::Utc> = time.into();
        facts.push(format!("modify={}", dt.format("%Y%m%d%H%M%S")));
    }
    
    facts.push(format!("UNIX.owner={}", owner));
    facts.push(format!("UNIX.group={}", owner));

    format!("{};", facts.join(";"))
}

fn convert_lf_to_crlf(data: &[u8]) -> Vec<u8> {
    let mut result = Vec::with_capacity(data.len() + data.len() / 10);
    let mut i = 0;
    while i < data.len() {
        if data[i] == b'\n' {
            if i == 0 || data[i - 1] != b'\r' {
                result.push(b'\r');
            }
            result.push(b'\n');
        } else {
            result.push(data[i]);
        }
        i += 1;
    }
    result
}

fn convert_crlf_to_lf(data: &[u8]) -> Vec<u8> {
    let mut result = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        if i + 1 < data.len() && data[i] == b'\r' && data[i + 1] == b'\n' {
            result.push(b'\n');
            i += 2;
        } else if data[i] == b'\r' {
            i += 1;
        } else {
            result.push(data[i]);
            i += 1;
        }
    }
    result
}
