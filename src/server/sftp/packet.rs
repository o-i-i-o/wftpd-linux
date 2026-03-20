use anyhow::Result;

pub const SSH_FX_OK: u32 = 0;
pub const SSH_FX_EOF: u32 = 1;
pub const SSH_FX_NO_SUCH_FILE: u32 = 2;
pub const SSH_FX_PERMISSION_DENIED: u32 = 3;
pub const SSH_FX_FAILURE: u32 = 4;

#[allow(dead_code)]
pub const SSH_FX_BAD_MESSAGE: u32 = 5;

#[allow(dead_code)]
pub const SSH_FX_NO_CONNECTION: u32 = 6;

#[allow(dead_code)]
pub const SSH_FX_CONNECTION_LOST: u32 = 7;
pub const SSH_FX_OP_UNSUPPORTED: u32 = 8;

pub fn parse_u32(data: &[u8], offset: usize) -> u32 {
    if offset + 4 > data.len() {
        return 0;
    }
    u32::from_be_bytes([data[offset], data[offset + 1], data[offset + 2], data[offset + 3]])
}

pub fn parse_u32_checked(data: &[u8], offset: usize) -> Result<u32> {
    if offset + 4 > data.len() {
        anyhow::bail!("Insufficient data for u32 at offset {}", offset);
    }
    Ok(u32::from_be_bytes([data[offset], data[offset + 1], data[offset + 2], data[offset + 3]]))
}

#[allow(dead_code)]
pub fn parse_u64_checked(data: &[u8], offset: usize) -> Result<u64> {
    if offset + 8 > data.len() {
        anyhow::bail!("Insufficient data for u64 at offset {}", offset);
    }
    Ok(u64::from_be_bytes([
        data[offset], data[offset + 1], data[offset + 2], data[offset + 3],
        data[offset + 4], data[offset + 5], data[offset + 6], data[offset + 7],
    ]))
}

pub fn parse_string(data: &[u8], offset: usize) -> Result<String> {
    if offset + 4 > data.len() {
        return Ok(String::new());
    }
    let len = parse_u32(data, offset) as usize;
    if offset + 4 + len > data.len() {
        return Ok(String::new());
    }
    Ok(String::from_utf8_lossy(&data[offset + 4..offset + 4 + len]).to_string())
}

#[allow(dead_code)]
pub fn parse_string_checked(data: &[u8], offset: usize) -> Result<(String, usize)> {
    if offset + 4 > data.len() {
        anyhow::bail!("Insufficient data for string length at offset {}", offset);
    }
    let len = u32::from_be_bytes([data[offset], data[offset + 1], data[offset + 2], data[offset + 3]]) as usize;
    if offset + 4 + len > data.len() {
        anyhow::bail!("String data exceeds buffer at offset {}", offset);
    }
    let raw = &data[offset + 4..offset + 4 + len];
    let s = std::str::from_utf8(raw)
        .map(|s| s.to_string())
        .unwrap_or_else(|_| {
            String::from_utf8_lossy(raw).into_owned()
        });
    Ok((s, len))
}

pub fn build_packet(payload: &[u8]) -> Vec<u8> {
    let mut packet = Vec::new();
    packet.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    packet.extend_from_slice(payload);
    packet
}

pub fn build_status_packet(id: u32, status: u32, msg: &str, lang: &str) -> Vec<u8> {
    let mut payload = vec![101];
    payload.extend_from_slice(&id.to_be_bytes());
    payload.extend_from_slice(&status.to_be_bytes());
    payload.extend_from_slice(&(msg.len() as u32).to_be_bytes());
    payload.extend_from_slice(msg.as_bytes());
    payload.extend_from_slice(&(lang.len() as u32).to_be_bytes());
    payload.extend_from_slice(lang.as_bytes());
    build_packet(&payload)
}

pub fn build_handle_packet(id: u32, handle: &str) -> Vec<u8> {
    let mut payload = vec![102];
    payload.extend_from_slice(&id.to_be_bytes());
    payload.extend_from_slice(&(handle.len() as u32).to_be_bytes());
    payload.extend_from_slice(handle.as_bytes());
    build_packet(&payload)
}

pub fn build_data_packet(id: u32, data: &[u8]) -> Vec<u8> {
    let mut payload = vec![103];
    payload.extend_from_slice(&id.to_be_bytes());
    payload.extend_from_slice(&(data.len() as u32).to_be_bytes());
    payload.extend_from_slice(data);
    build_packet(&payload)
}

pub fn build_attrs(is_dir: bool, size: u64) -> Vec<u8> {
    let mut attrs = Vec::new();
    let flags: u32 = 0x00000001 | 0x00000002 | 0x00000004 | 0x00000008 | 0x00000010;
    attrs.extend_from_slice(&flags.to_be_bytes());
    attrs.extend_from_slice(&size.to_be_bytes());
    let uid: u32 = 1000;
    let gid: u32 = 1000;
    attrs.extend_from_slice(&uid.to_be_bytes());
    attrs.extend_from_slice(&gid.to_be_bytes());
    let permissions = if is_dir {
        0o40755u32
    } else {
        0o100644u32
    };
    attrs.extend_from_slice(&permissions.to_be_bytes());
    let atime: u32 = 0;
    let mtime: u32 = 0;
    attrs.extend_from_slice(&atime.to_be_bytes());
    attrs.extend_from_slice(&mtime.to_be_bytes());
    attrs
}

#[allow(dead_code)]
pub fn build_attrs_from_metadata(metadata: &std::fs::Metadata) -> Vec<u8> {
    let mut attrs = Vec::new();
    let flags: u32 = 0x00000001 | 0x00000002 | 0x00000004 | 0x00000008 | 0x00000010;
    attrs.extend_from_slice(&flags.to_be_bytes());
    attrs.extend_from_slice(&metadata.len().to_be_bytes());
    
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        attrs.extend_from_slice(&metadata.uid().to_be_bytes());
        attrs.extend_from_slice(&metadata.gid().to_be_bytes());
        attrs.extend_from_slice(&metadata.mode().to_be_bytes());
    }
    
    #[cfg(not(unix))]
    {
        let uid: u32 = 1000;
        let gid: u32 = 1000;
        attrs.extend_from_slice(&uid.to_be_bytes());
        attrs.extend_from_slice(&gid.to_be_bytes());
        let permissions = if metadata.is_dir() {
            0o40755u32
        } else {
            0o100644u32
        };
        attrs.extend_from_slice(&permissions.to_be_bytes());
    }
    
    let atime: u32 = metadata.accessed()
        .map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as u32)
        .unwrap_or(0);
    let mtime: u32 = metadata.modified()
        .map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as u32)
        .unwrap_or(0);
    attrs.extend_from_slice(&atime.to_be_bytes());
    attrs.extend_from_slice(&mtime.to_be_bytes());
    attrs
}
