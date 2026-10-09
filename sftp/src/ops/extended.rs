//! openssh 协议扩展的辅助实现：md5sum / sha256sum（摘要校验）与
//! space-available（磁盘空间查询）。协议分发入口在 [`super::protocol`]。

use std::io::SeekFrom;
use std::path::PathBuf;

use russh_sftp::protocol::{Packet, StatusCode};

use super::{SftpFileHandler, SftpResult};

pub(super) enum Digest {
    Md5,
    Sha256,
}

impl SftpFileHandler {
    /// md5sum/sha256sum@openssh.com：请求体为 (path, offset, length)
    pub(super) async fn extended_digest(
        &self,
        id: u32,
        data: &[u8],
        kind: Digest,
    ) -> SftpResult<Packet> {
        let (path, offset, length) = parse_digest_request(data).ok_or(StatusCode::BadMessage)?;
        let resolved = self.resolve(&path)?;

        let payload = digest_file(&resolved, offset, length, kind)
            .await
            .map_err(|e| Self::io_status(&e))?;

        Ok(Packet::ExtendedReply(russh_sftp::protocol::ExtendedReply {
            id,
            data: payload,
        }))
    }

    /// space-available@openssh.com（statvfs 编码按 draft-ietf-secsh-filexfer-06 §8）
    pub(super) fn extended_space_available(&self, id: u32, data: &[u8]) -> SftpResult<Packet> {
        let bad = StatusCode::BadMessage;
        let path_len =
            u32::from_be_bytes(data.get(0..4).ok_or(bad)?.try_into().map_err(|_| bad)?) as usize;
        let path = std::str::from_utf8(data.get(4..4 + path_len).ok_or(bad)?).map_err(|_| bad)?;
        let resolved = self.resolve(path)?;

        let stats = nix::sys::statvfs::statvfs(&resolved).map_err(|_| StatusCode::Failure)?;

        let mut payload = Vec::with_capacity(88);
        for v in [
            stats.block_size() as u64,
            stats.fragment_size() as u64,
            stats.blocks() as u64,
            stats.blocks_free() as u64,
            stats.blocks_available() as u64,
            stats.files() as u64,
            stats.files_free() as u64,
            stats.files_available() as u64,
            0u64, // fsid
            0u64, // flags
            stats.name_max() as u64,
        ] {
            payload.extend_from_slice(&v.to_be_bytes());
        }

        Ok(Packet::ExtendedReply(russh_sftp::protocol::ExtendedReply {
            id,
            data: payload,
        }))
    }
}

/// 解析 md5sum/sha256sum 请求：string path, uint64 offset, uint64 length
/// （length == 0 表示从 offset 起到文件末尾）
pub(super) fn parse_digest_request(data: &[u8]) -> Option<(String, u64, u64)> {
    let read_u32 = |pos: usize| -> Option<u32> {
        Some(u32::from_be_bytes(data.get(pos..pos + 4)?.try_into().ok()?))
    };
    let read_u64 = |pos: usize| -> Option<u64> {
        Some(u64::from_be_bytes(data.get(pos..pos + 8)?.try_into().ok()?))
    };

    let path_len = read_u32(0)? as usize;
    let path = String::from_utf8(data.get(4..4 + path_len)?.to_vec()).ok()?;
    let mut pos = 4 + path_len;
    let offset = read_u64(pos).unwrap_or(0);
    pos += 8;
    let length = read_u64(pos).unwrap_or(0);
    Some((path, offset, length))
}

async fn digest_file(
    path: &PathBuf,
    offset: u64,
    length: u64,
    kind: Digest,
) -> std::io::Result<Vec<u8>> {
    use md5::Digest as _;
    use tokio::io::{AsyncReadExt, AsyncSeekExt};

    let mut file = tokio::fs::File::open(path).await?;
    file.seek(SeekFrom::Start(offset)).await?;

    let mut remaining = length;
    let mut buf = [0u8; 8192];
    let mut hasher = match kind {
        Digest::Md5 => Box::new(md5::Md5::new()) as Box<dyn md5::digest::DynDigest + Send>,
        Digest::Sha256 => Box::new(sha2::Sha256::new()) as Box<dyn md5::digest::DynDigest + Send>,
    };
    loop {
        let want = if remaining == 0 {
            buf.len()
        } else {
            buf.len()
                .min(usize::try_from(remaining).unwrap_or(usize::MAX))
        };
        let n = file.read(&mut buf[..want]).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        if remaining > 0 {
            remaining -= n as u64;
            if remaining == 0 {
                break;
            }
        }
    }

    Ok(hex::encode(hasher.finalize()).into_bytes())
}
