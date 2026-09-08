use anyhow::{Result, bail};
use std::path::{Path, PathBuf};
use tracing::{debug, error, warn};

use crate::error::{WftpgError, WftpgResult};

const MAX_PATH_LENGTH: usize = 4096;

#[must_use]
pub fn real_to_virtual_path(real_path: &str, home_dir: &str) -> String {
    let home_canon = match Path::new(home_dir).canonicalize() {
        Ok(c) => c,
        Err(_) => Path::new(home_dir).to_path_buf(),
    };

    let real_path_buf = Path::new(real_path);

    if real_path_buf == home_canon {
        return "/".to_string();
    }

    if let Ok(relative) = real_path_buf.strip_prefix(&home_canon) {
        let relative_str = relative.to_string_lossy();
        if relative_str.is_empty() {
            "/".to_string()
        } else {
            format!("/{}", relative_str.replace('\\', "/"))
        }
    } else {
        real_path.replace('\\', "/")
    }
}

#[must_use]
pub fn virtual_to_real_path(virtual_path: &str, home_dir: &str) -> String {
    let home_canon = match Path::new(home_dir).canonicalize() {
        Ok(c) => c.to_string_lossy().to_string(),
        Err(_) => home_dir.to_string(),
    };

    let clean_virtual = virtual_path.trim();

    if clean_virtual.is_empty() || clean_virtual == "/" {
        return home_canon;
    }

    if clean_virtual.starts_with('/') {
        let relative = clean_virtual.trim_start_matches('/');
        format!("{}/{}", home_canon.trim_end_matches('/'), relative)
    } else {
        format!("{}/{}", home_canon.trim_end_matches('/'), clean_virtual)
    }
}

#[must_use]
pub fn is_safe_username(username: &str) -> bool {
    if username.is_empty() || username.len() > 64 {
        return false;
    }
    let bytes = username.as_bytes();
    let first = bytes[0];
    if !first.is_ascii_alphanumeric() && first != b'_' {
        return false;
    }
    bytes
        .iter()
        .all(|&c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}

fn check_path_length(path: &str) -> WftpgResult<()> {
    if path.len() > MAX_PATH_LENGTH {
        return Err(WftpgError::PathResolveError(format!(
            "路径过长: {} 字节 (最大 {})",
            path.len(),
            MAX_PATH_LENGTH
        )));
    }
    Ok(())
}

fn canonicalize_home(home_dir: &str) -> WftpgResult<PathBuf> {
    let home = PathBuf::from(home_dir);

    match home.canonicalize() {
        Ok(canon) => Ok(canon),
        Err(e) => {
            error!(home = ?home, error = %e, "无法规范化主目录");
            Err(WftpgError::PathResolveError(format!(
                "主目录不存在或无法访问：{}",
                home.display()
            )))
        }
    }
}

async fn canonicalize_home_async(home_dir: &str) -> WftpgResult<PathBuf> {
    let home = PathBuf::from(home_dir);

    match tokio::fs::canonicalize(&home).await {
        Ok(canon) => Ok(canon),
        Err(e) => {
            error!(home = ?home, error = %e, "无法规范化主目录");
            Err(WftpgError::PathResolveError(format!(
                "主目录不存在或无法访问：{}",
                home.display()
            )))
        }
    }
}

fn apply_path_components_safe(
    base: PathBuf,
    home_canon: &Path,
    clean_path: &str,
    original_path: &str,
) -> WftpgResult<PathBuf> {
    let mut safe_path = base;

    for component in Path::new(clean_path).components() {
        match component {
            std::path::Component::Normal(name) => {
                safe_path.push(name);
            }
            std::path::Component::ParentDir => {
                if safe_path != home_canon && safe_path.starts_with(home_canon) {
                    if !safe_path.pop() {
                        warn!("路径遍历攻击: {:?} 中包含过多的父目录", original_path);
                        return Err(WftpgError::PathResolveError(
                            "路径中包含过多的父目录".to_string(),
                        ));
                    }
                } else {
                    warn!(
                        "路径遍历攻击被阻止: {:?} 中无法访问主目录之上的目录",
                        original_path
                    );
                    return Err(WftpgError::PathResolveError(
                        "无法访问主目录之上的目录".to_string(),
                    ));
                }
            }
            std::path::Component::RootDir => {
                if clean_path.starts_with('/') {
                    safe_path = home_canon.to_path_buf();
                }
            }
            std::path::Component::CurDir | std::path::Component::Prefix(_) => {}
        }
    }

    if safe_path.starts_with(home_canon) {
        if safe_path.exists() {
            match safe_path.canonicalize() {
                Ok(canon) => {
                    if canon.starts_with(home_canon) {
                        Ok(canon)
                    } else {
                        warn!(
                            "路径遍历攻击被阻止: {:?} 逃逸了主目录 {:?}",
                            canon, home_canon
                        );
                        Err(WftpgError::PathResolveError("路径逃逸了主目录".to_string()))
                    }
                }
                Err(e) => {
                    warn!("无法规范化路径 {:?}: {}", safe_path, e);
                    Err(WftpgError::PathResolveError("无法访问路径".to_string()))
                }
            }
        } else {
            Ok(safe_path)
        }
    } else {
        warn!(
            "路径遍历攻击被阻止: {:?} 逃逸了主目录 {:?}",
            safe_path, home_canon
        );
        Err(WftpgError::PathResolveError("路径逃逸了主目录".to_string()))
    }
}

async fn apply_path_components_safe_async(
    base: PathBuf,
    home_canon: &Path,
    clean_path: &str,
    original_path: &str,
) -> WftpgResult<PathBuf> {
    let mut safe_path = base;

    for component in Path::new(clean_path).components() {
        match component {
            std::path::Component::Normal(name) => {
                safe_path.push(name);
            }
            std::path::Component::ParentDir => {
                if safe_path != home_canon && safe_path.starts_with(home_canon) {
                    if !safe_path.pop() {
                        warn!("路径遍历攻击: {:?} 中包含过多的父目录", original_path);
                        return Err(WftpgError::PathResolveError(
                            "路径中包含过多的父目录".to_string(),
                        ));
                    }
                } else {
                    warn!(
                        "路径遍历攻击被阻止: {:?} 中无法访问主目录之上的目录",
                        original_path
                    );
                    return Err(WftpgError::PathResolveError(
                        "无法访问主目录之上的目录".to_string(),
                    ));
                }
            }
            std::path::Component::RootDir => {
                if clean_path.starts_with('/') {
                    safe_path = home_canon.to_path_buf();
                }
            }
            std::path::Component::CurDir | std::path::Component::Prefix(_) => {}
        }
    }

    if safe_path.starts_with(home_canon) {
        match tokio::fs::try_exists(&safe_path).await {
            Ok(true) => match tokio::fs::canonicalize(&safe_path).await {
                Ok(canon) => {
                    if canon.starts_with(home_canon) {
                        Ok(canon)
                    } else {
                        warn!(
                            "路径遍历攻击被阻止: {:?} 逃逸了主目录 {:?}",
                            canon, home_canon
                        );
                        Err(WftpgError::PathResolveError("路径逃逸了主目录".to_string()))
                    }
                }
                Err(e) => {
                    warn!("无法规范化路径 {:?}: {}", safe_path, e);
                    Err(WftpgError::PathResolveError("无法访问路径".to_string()))
                }
            },
            Ok(false) => Ok(safe_path),
            Err(e) => {
                warn!("无法检查路径是否存在 {:?}: {}", safe_path, e);
                Err(WftpgError::PathResolveError("无法访问路径".to_string()))
            }
        }
    } else {
        warn!(
            "路径遍历攻击被阻止: {:?} 逃逸了主目录 {:?}",
            safe_path, home_canon
        );
        Err(WftpgError::PathResolveError("路径逃逸了主目录".to_string()))
    }
}

fn resolve_nonexistent_path(
    resolved: &Path,
    home_canon: &Path,
    original_path: &str,
) -> WftpgResult<PathBuf> {
    let clean_path = original_path.trim();

    if Path::new(clean_path).is_absolute() {
        if resolved.starts_with(home_canon) {
            Ok(resolved.to_path_buf())
        } else {
            warn!("绝对路径位于主目录之外: {:?}", resolved);
            Err(WftpgError::PathResolveError(
                "绝对路径位于主目录之外".to_string(),
            ))
        }
    } else {
        apply_path_components_safe(
            home_canon.to_path_buf(),
            home_canon,
            clean_path,
            original_path,
        )
    }
}

async fn resolve_nonexistent_path_async(
    resolved: &Path,
    home_canon: &Path,
    original_path: &str,
) -> WftpgResult<PathBuf> {
    let clean_path = original_path.trim();

    if Path::new(clean_path).is_absolute() {
        if resolved.starts_with(home_canon) {
            Ok(resolved.to_path_buf())
        } else {
            warn!("绝对路径位于主目录之外: {:?}", resolved);
            Err(WftpgError::PathResolveError(
                "绝对路径位于主目录之外".to_string(),
            ))
        }
    } else {
        apply_path_components_safe_async(
            home_canon.to_path_buf(),
            home_canon,
            clean_path,
            original_path,
        )
        .await
    }
}

/// 将用户提供的路径解析为主目录内的绝对路径（不跟随未存在的路径）
///
/// # Errors
/// 主目录不存在或无法访问、路径超过最大长度，或路径（含符号链接解析结果）
/// 逃逸主目录时返回错误
pub fn safe_resolve_path(home_dir: &str, path: &str) -> WftpgResult<PathBuf> {
    check_path_length(path)?;

    let home_canon = canonicalize_home(home_dir)?;

    let clean_path = path.trim();

    if clean_path.is_empty() || clean_path == "." || clean_path == "./" {
        return Ok(home_canon);
    }

    let clean_path = if let Some(stripped) = clean_path.strip_prefix("./") {
        stripped
    } else {
        clean_path
    };

    if clean_path.is_empty() {
        return Ok(home_canon);
    }

    let resolved = if Path::new(clean_path).is_absolute() {
        let relative = clean_path.trim_start_matches('/');
        if relative.is_empty() {
            home_canon.clone()
        } else {
            home_canon.join(relative)
        }
    } else {
        home_canon.join(clean_path)
    };

    match resolved.canonicalize() {
        Ok(canon) => {
            if canon.starts_with(&home_canon) {
                Ok(canon)
            } else {
                warn!(
                    "路径遍历攻击被阻止: {:?} 位于主目录 {:?} 之外",
                    canon, home_canon
                );
                Err(WftpgError::PathResolveError(
                    "路径位于主目录之外".to_string(),
                ))
            }
        }
        Err(_) => resolve_nonexistent_path(&resolved, &home_canon, path),
    }
}

/// [`safe_resolve_path`] 的异步版本
///
/// # Errors
/// 同 [`safe_resolve_path`]
pub async fn safe_resolve_path_async(home_dir: &str, path: &str) -> WftpgResult<PathBuf> {
    check_path_length(path)?;

    let home_canon = canonicalize_home_async(home_dir).await?;

    let clean_path = path.trim();

    if clean_path.is_empty() || clean_path == "." || clean_path == "./" {
        return Ok(home_canon);
    }

    let clean_path = if let Some(stripped) = clean_path.strip_prefix("./") {
        stripped
    } else {
        clean_path
    };

    if clean_path.is_empty() {
        return Ok(home_canon);
    }

    let resolved = if Path::new(clean_path).is_absolute() {
        let relative = clean_path.trim_start_matches('/');
        if relative.is_empty() {
            home_canon.clone()
        } else {
            home_canon.join(relative)
        }
    } else {
        home_canon.join(clean_path)
    };

    match tokio::fs::canonicalize(&resolved).await {
        Ok(canon) => {
            if canon.starts_with(&home_canon) {
                Ok(canon)
            } else {
                warn!(
                    "路径遍历攻击被阻止: {:?} 位于主目录 {:?} 之外",
                    canon, home_canon
                );
                Err(WftpgError::PathResolveError(
                    "路径位于主目录之外".to_string(),
                ))
            }
        }
        Err(_) => resolve_nonexistent_path_async(&resolved, &home_canon, path).await,
    }
}

/// 相对路径以 `cwd` 为基准的 [`safe_resolve_path`] 变体
///
/// # Errors
/// 同 [`safe_resolve_path`]；另外当 `cwd` 位于主目录之外或无法访问时也返回错误
pub fn safe_resolve_path_with_cwd(cwd: &str, home_dir: &str, path: &str) -> WftpgResult<PathBuf> {
    check_path_length(path)?;

    let home_canon = canonicalize_home(home_dir)?;

    let clean_path = path.trim();

    if clean_path.is_empty() || clean_path == "." || clean_path == "./" {
        return resolve_cwd(cwd, &home_canon);
    }

    let clean_path = if let Some(stripped) = clean_path.strip_prefix("./") {
        stripped
    } else {
        clean_path
    };

    if clean_path.is_empty() {
        return resolve_cwd(cwd, &home_canon);
    }

    let resolved = if Path::new(clean_path).is_absolute() {
        let relative = clean_path.trim_start_matches('/');
        if relative.is_empty() {
            home_canon.clone()
        } else {
            home_canon.join(relative)
        }
    } else {
        Path::new(cwd).join(clean_path)
    };

    match resolved.canonicalize() {
        Ok(canon) => {
            if canon.starts_with(&home_canon) {
                Ok(canon)
            } else {
                warn!(
                    "路径遍历攻击被阻止: {:?} 位于主目录 {:?} 之外",
                    canon, home_canon
                );
                Err(WftpgError::PathResolveError(
                    "路径位于主目录之外".to_string(),
                ))
            }
        }
        Err(_) => resolve_nonexistent_path_with_cwd(&resolved, &home_canon, cwd, path),
    }
}

/// [`safe_resolve_path_with_cwd`] 的异步版本
///
/// # Errors
/// 同 [`safe_resolve_path_with_cwd`]
pub async fn safe_resolve_path_with_cwd_async(
    cwd: &str,
    home_dir: &str,
    path: &str,
) -> WftpgResult<PathBuf> {
    check_path_length(path)?;

    let home_canon = canonicalize_home_async(home_dir).await?;

    let clean_path = path.trim();

    if clean_path.is_empty() || clean_path == "." || clean_path == "./" {
        return resolve_cwd_async(cwd, &home_canon).await;
    }

    let clean_path = if let Some(stripped) = clean_path.strip_prefix("./") {
        stripped
    } else {
        clean_path
    };

    if clean_path.is_empty() {
        return resolve_cwd_async(cwd, &home_canon).await;
    }

    let resolved = if Path::new(clean_path).is_absolute() {
        let relative = clean_path.trim_start_matches('/');
        if relative.is_empty() {
            home_canon.clone()
        } else {
            home_canon.join(relative)
        }
    } else {
        Path::new(cwd).join(clean_path)
    };

    match tokio::fs::canonicalize(&resolved).await {
        Ok(canon) => {
            if canon.starts_with(&home_canon) {
                Ok(canon)
            } else {
                warn!(
                    "路径遍历攻击被阻止: {:?} 位于主目录 {:?} 之外",
                    canon, home_canon
                );
                Err(WftpgError::PathResolveError(
                    "路径位于主目录之外".to_string(),
                ))
            }
        }
        Err(_) => resolve_nonexistent_path_with_cwd_async(&resolved, &home_canon, cwd, path).await,
    }
}

fn resolve_nonexistent_path_with_cwd(
    resolved: &Path,
    home_canon: &Path,
    cwd: &str,
    original_path: &str,
) -> WftpgResult<PathBuf> {
    let clean_path = original_path.trim();

    if Path::new(clean_path).is_absolute() {
        if resolved.starts_with(home_canon) {
            Ok(resolved.to_path_buf())
        } else {
            warn!("绝对路径位于主目录之外: {:?}", resolved);
            Err(WftpgError::PathResolveError(
                "绝对路径位于主目录之外".to_string(),
            ))
        }
    } else {
        let cwd_canon = resolve_cwd(cwd, home_canon)?;
        apply_path_components_safe(cwd_canon, home_canon, clean_path, original_path)
    }
}

async fn resolve_nonexistent_path_with_cwd_async(
    resolved: &Path,
    home_canon: &Path,
    cwd: &str,
    original_path: &str,
) -> WftpgResult<PathBuf> {
    let clean_path = original_path.trim();

    if Path::new(clean_path).is_absolute() {
        if resolved.starts_with(home_canon) {
            Ok(resolved.to_path_buf())
        } else {
            warn!("绝对路径位于主目录之外: {:?}", resolved);
            Err(WftpgError::PathResolveError(
                "绝对路径位于主目录之外".to_string(),
            ))
        }
    } else {
        let cwd_canon = resolve_cwd_async(cwd, home_canon).await?;
        apply_path_components_safe_async(cwd_canon, home_canon, clean_path, original_path).await
    }
}

fn resolve_cwd(cwd: &str, home_canon: &Path) -> WftpgResult<PathBuf> {
    if cwd.is_empty() {
        return Ok(home_canon.to_path_buf());
    }

    let cwd_path = PathBuf::from(cwd);
    match cwd_path.canonicalize() {
        Ok(canon) => {
            if canon.starts_with(home_canon) {
                Ok(canon)
            } else {
                warn!("当前工作目录位于主目录之外: {:?}", cwd_path);
                Err(WftpgError::PathResolveError(
                    "当前工作目录位于主目录之外".to_string(),
                ))
            }
        }
        Err(e) => {
            warn!("当前工作目录不存在或无法访问 {:?}: {}", cwd_path, e);
            Err(WftpgError::PathResolveError(
                "当前工作目录不存在或无法访问".to_string(),
            ))
        }
    }
}

async fn resolve_cwd_async(cwd: &str, home_canon: &Path) -> WftpgResult<PathBuf> {
    if cwd.is_empty() {
        return Ok(home_canon.to_path_buf());
    }

    let cwd_path = PathBuf::from(cwd);
    match tokio::fs::canonicalize(&cwd_path).await {
        Ok(canon) => {
            if canon.starts_with(home_canon) {
                Ok(canon)
            } else {
                warn!("当前工作目录位于主目录之外: {:?}", cwd_path);
                Err(WftpgError::PathResolveError(
                    "当前工作目录位于主目录之外".to_string(),
                ))
            }
        }
        Err(e) => {
            warn!("当前工作目录不存在或无法访问 {:?}: {}", cwd_path, e);
            Err(WftpgError::PathResolveError(
                "当前工作目录不存在或无法访问".to_string(),
            ))
        }
    }
}

#[must_use]
pub fn get_file_mtime(metadata: &std::fs::Metadata) -> String {
    use chrono::DateTime;
    use std::time::UNIX_EPOCH;

    if let Ok(system_time) = metadata.modified()
        && let Ok(duration) = system_time.duration_since(UNIX_EPOCH)
    {
        let secs = i64::try_from(duration.as_secs()).unwrap_or(i64::MAX);
        let nanos = duration.subsec_nanos();
        if let Some(dt) = DateTime::from_timestamp(secs, nanos) {
            return dt.format("%Y-%m-%d %H:%M").to_string();
        }
    }
    "1970-01-01 00:00".to_string()
}

#[must_use]
pub fn get_file_mtime_raw(metadata: &std::fs::Metadata) -> String {
    use std::time::UNIX_EPOCH;
    if let Ok(time) = metadata.modified()
        && let Ok(duration) = time.duration_since(UNIX_EPOCH)
    {
        return format!("{}", duration.as_secs());
    }
    "0".to_string()
}

#[must_use]
pub fn escape_mlst_filename(name: &str) -> String {
    use std::fmt::Write as _;

    let mut result = String::with_capacity(name.len());
    for c in name.chars() {
        match c {
            '\\' => result.push_str("\\\\"),
            ';' => result.push_str("\\;"),
            '=' => result.push_str("\\="),
            ' ' => result.push_str("\\ "),
            '\n' => result.push_str("\\012"),
            '\r' => result.push_str("\\015"),
            '\t' => result.push_str("\\011"),
            c if c.is_control() => {
                if let Ok(byte) = u8::try_from(u32::from(c)) {
                    let _ = write!(result, "\\{byte:03o}");
                } else {
                    result.push('?');
                }
            }
            c => result.push(c),
        }
    }
    result
}

#[must_use]
pub fn format_mtime_rfc3659(metadata: &std::fs::Metadata) -> String {
    use chrono::DateTime;
    use std::time::UNIX_EPOCH;

    if let Ok(system_time) = metadata.modified()
        && let Ok(duration) = system_time.duration_since(UNIX_EPOCH)
    {
        let secs = i64::try_from(duration.as_secs()).unwrap_or(i64::MAX);
        let nanos = duration.subsec_nanos();
        if let Some(dt) = DateTime::from_timestamp(secs, nanos) {
            return dt.format("%Y%m%d%H%M%S%.3f").to_string();
        }
    }
    "19700101000000".to_string()
}

#[must_use]
pub fn get_unix_mode(metadata: &std::fs::Metadata) -> String {
    use std::os::unix::fs::PermissionsExt;
    let mode = metadata.permissions().mode();
    format!("{:04o}", mode & 0o7777)
}

#[must_use]
pub fn build_mlst_facts(metadata: &std::fs::Metadata) -> String {
    let mut facts: Vec<String> = Vec::new();

    if metadata.is_dir() {
        facts.push("type=dir;".to_string());
    } else {
        facts.push("type=file;".to_string());
    }

    facts.push(format!("size={};", metadata.len()));

    let mtime = format_mtime_rfc3659(metadata);
    facts.push(format!("modify={mtime};"));

    let mode = get_unix_mode(metadata);
    facts.push(format!("unix.mode={mode};"));

    facts.join("")
}

#[cfg(unix)]
/// 以 `openat(O_NOFOLLOW)` 逐段打开主目录下的相对路径，防符号链接竞态
///
/// # Errors
/// 主目录无法规范化或打开、相对路径包含 `..`、任一路径组件打开失败
/// （包括组件为符号链接被 `O_NOFOLLOW` 拒绝）时返回错误
pub fn safe_open_file_at(home_dir: &str, relative_path: &str) -> WftpgResult<std::fs::File> {
    use nix::fcntl::{AT_FDCWD, OFlag, openat};
    use nix::sys::stat::Mode;
    use std::os::fd::AsFd;

    let home_canon = canonicalize_home(home_dir)?;

    let dirfd = openat(
        AT_FDCWD,
        &home_canon,
        OFlag::O_DIRECTORY | OFlag::O_RDONLY,
        Mode::empty(),
    )
    .map_err(|e| {
        error!("无法打开主目录 {:?}: {}", home_canon, e);
        WftpgError::PathResolveError("无法打开主目录".to_string())
    })?;

    let components: Vec<&str> = relative_path.trim_matches('/').split('/').collect();
    let mut current_fd = dirfd;
    let mut fds_to_close: Vec<_> = Vec::new();

    for (i, component) in components.iter().enumerate() {
        if *component == ".." {
            warn!("路径遍历攻击被阻止: 相对路径中包含 '..'");
            for fd in fds_to_close {
                nix::unistd::close(fd).ok();
            }
            nix::unistd::close(current_fd).ok();
            return Err(WftpgError::PathResolveError(
                "路径中不允许包含父目录引用".to_string(),
            ));
        }

        if *component == "." || component.is_empty() {
            continue;
        }

        let is_last = i == components.len() - 1;
        let flags = if is_last {
            OFlag::O_RDONLY | OFlag::O_NOFOLLOW
        } else {
            OFlag::O_DIRECTORY | OFlag::O_RDONLY | OFlag::O_NOFOLLOW
        };

        match openat(current_fd.as_fd(), *component, flags, Mode::empty()) {
            Ok(fd) => {
                fds_to_close.push(current_fd);
                current_fd = fd;
            }
            Err(e) => {
                warn!("无法打开路径组件 '{}': {}", component, e);
                for fd in fds_to_close {
                    nix::unistd::close(fd).ok();
                }
                nix::unistd::close(current_fd).ok();
                return Err(WftpgError::PathResolveError(format!(
                    "无法访问路径组件: {component}"
                )));
            }
        }
    }

    for fd in fds_to_close {
        nix::unistd::close(fd).ok();
    }

    Ok(std::fs::File::from(current_fd))
}

#[cfg(unix)]
/// [`safe_open_file_at`] 的异步版本
///
/// # Errors
/// 同 [`safe_open_file_at`]
pub async fn safe_open_file_at_async(
    home_dir: &str,
    relative_path: &str,
) -> WftpgResult<tokio::fs::File> {
    use nix::fcntl::{AT_FDCWD, OFlag, openat};
    use nix::sys::stat::Mode;
    use std::os::fd::AsFd;

    let home_canon = canonicalize_home_async(home_dir).await?;

    let dirfd = openat(
        AT_FDCWD,
        &home_canon,
        OFlag::O_DIRECTORY | OFlag::O_RDONLY,
        Mode::empty(),
    )
    .map_err(|e| {
        error!("无法打开主目录 {:?}: {}", home_canon, e);
        WftpgError::PathResolveError("无法打开主目录".to_string())
    })?;

    let components: Vec<&str> = relative_path.trim_matches('/').split('/').collect();
    let mut current_fd = dirfd;
    let mut fds_to_close: Vec<_> = Vec::new();

    for (i, component) in components.iter().enumerate() {
        if *component == ".." {
            warn!("路径遍历攻击被阻止：相对路径中包含 '..'");
            for fd in fds_to_close {
                nix::unistd::close(fd).ok();
            }
            nix::unistd::close(current_fd).ok();
            return Err(WftpgError::PathResolveError(
                "路径中不允许包含父目录引用".to_string(),
            ));
        }

        if *component == "." || component.is_empty() {
            continue;
        }

        let is_last = i == components.len() - 1;
        // 关键安全改进：始终使用 O_NOFOLLOW 防止符号链接攻击
        let flags = if is_last {
            OFlag::O_RDONLY | OFlag::O_NOFOLLOW
        } else {
            OFlag::O_DIRECTORY | OFlag::O_RDONLY | OFlag::O_NOFOLLOW
        };

        match openat(current_fd.as_fd(), *component, flags, Mode::empty()) {
            Ok(fd) => {
                fds_to_close.push(current_fd);
                current_fd = fd;
            }
            Err(e) => {
                warn!("无法打开路径组件 '{}': {}", component, e);
                for fd in fds_to_close {
                    nix::unistd::close(fd).ok();
                }
                nix::unistd::close(current_fd).ok();
                return Err(WftpgError::PathResolveError(format!(
                    "无法访问路径组件：{component}"
                )));
            }
        }
    }

    for fd in fds_to_close {
        nix::unistd::close(fd).ok();
    }

    let std_file = std::fs::File::from(current_fd);
    Ok(tokio::fs::File::from_std(std_file))
}

/// 🔒 严格验证路径是否在 chroot 监狱内
///
/// 这个函数比 `safe_resolve_path` 更严格，它会：
/// 1. canonicalize 路径（解析所有符号链接）
/// 2. 检查规范化后的路径是否在 home 目录内
/// 3. 逐段检查路径组件，防止符号链接逃逸
///
/// # Arguments
/// * `path` - 要验证的路径
/// * `home_dir` - chroot 根目录（用户家目录）
///
/// # Returns
/// * `Ok(PathBuf)` - 规范化后的绝对路径
/// * `Err` - 如果路径逃逸或无效
///
/// canonicalize 全部符号链接，并逐段检查祖先目录，确保没有任何一级
/// 通过符号链接指向主目录之外。
///
/// # Errors
/// 主目录无法规范化、路径或其父目录逃逸 chroot、符号链接祖先指向 chroot
/// 之外，或读取符号链接失败时返回错误
pub async fn validate_path_within_chroot(path: &str, home_dir: &str) -> Result<PathBuf> {
    debug!("Validating path: {:?} within chroot: {:?}", path, home_dir);

    let home_canon = tokio::fs::canonicalize(home_dir)
        .await
        .map_err(|e| anyhow::anyhow!("Cannot canonicalize home directory: {e}"))?;

    // 首先解析路径
    let resolved = if Path::new(path).is_absolute() {
        // 对于绝对路径，去掉前导 / 并连接到 home 目录
        let relative = path.trim_start_matches('/');
        if relative.is_empty() {
            home_canon.clone()
        } else {
            home_canon.join(relative)
        }
    } else {
        home_canon.join(path)
    };

    // 关键：canonicalize 以解析所有符号链接
    let canon_path = match tokio::fs::canonicalize(&resolved).await {
        Ok(p) => p,
        Err(e) => {
            // 如果路径不存在，尝试部分 canonicalize
            debug!(
                "Path does not exist, attempting partial canonicalization: {}",
                e
            );
            // 对于不存在的路径，我们仍然可以检查其父目录
            if let Some(parent) = resolved.parent() {
                let parent_canon = tokio::fs::canonicalize(parent)
                    .await
                    .unwrap_or_else(|_| parent.to_path_buf());

                if !parent_canon.starts_with(&home_canon) {
                    bail!("Parent path escapes chroot: {}", parent_canon.display());
                }
            }
            // 返回原始解析路径（未完全 canonicalize）
            resolved
        }
    };

    // 严格检查：规范化后的路径必须在 home 目录内
    if !canon_path.starts_with(&home_canon) {
        warn!(
            "SECURITY: Path escape attempt detected! Resolved: {:?}, Home: {:?}",
            canon_path, home_canon
        );
        bail!("Path escapes chroot jail: {}", canon_path.display());
    }

    // 逐段检查祖先路径，确保没有符号链接指向外部
    let mut current: &Path = &canon_path;
    while let Some(ancestor) = current.parent() {
        if ancestor == home_canon {
            break;
        }

        // 检查这个祖先是否是符号链接
        if let Ok(meta) = tokio::fs::symlink_metadata(ancestor).await
            && meta.file_type().is_symlink()
        {
            let link_target = tokio::fs::read_link(ancestor).await?;
            debug!(
                "Checking symlink ancestor: {:?} -> {:?}",
                ancestor, link_target
            );

            // 如果符号链接目标是绝对的，必须在 home 内
            if link_target.is_absolute() && !link_target.starts_with(&home_canon) {
                bail!(
                    "Symlink ancestor {} points outside chroot to {}",
                    ancestor.display(),
                    link_target.display()
                );
            }
        }

        current = ancestor;
    }

    debug!("Path validation successful: {:?}", canon_path);
    Ok(canon_path)
}

/// 🔒 验证路径是否可以安全创建（用于 MKDIR、CREATE 等操作）
///
/// 检查新路径是否会在创建后导致安全问题
/// 验证新路径可以安全创建（用于 MKDIR、CREATE 等操作）
///
/// 检查所有已存在的祖先组件，确保创建动作不会落在符号链接之下。
///
/// # Errors
/// 主目录无法规范化、创建路径逃逸 chroot，或祖先中存在符号链接时返回错误
pub async fn validate_path_for_creation(path: &str, home_dir: &str) -> Result<PathBuf> {
    debug!(
        "Validating path for creation: {:?} within chroot: {:?}",
        path, home_dir
    );

    let home_canon = tokio::fs::canonicalize(home_dir)
        .await
        .map_err(|e| anyhow::anyhow!("Cannot canonicalize home directory: {e}"))?;

    // 解析目标路径
    let target_path = if Path::new(path).is_absolute() {
        let relative = path.trim_start_matches('/');
        if relative.is_empty() {
            bail!("Cannot create root directory");
        }
        home_canon.join(relative)
    } else {
        home_canon.join(path)
    };

    // 检查路径本身（即使不存在）
    if target_path.starts_with(&home_canon) {
        // 还需要检查所有存在的祖先组件
        for ancestor in target_path.ancestors() {
            if ancestor == target_path {
                continue; // 跳过目标本身（因为它不存在）
            }

            if let Ok(meta) = tokio::fs::metadata(ancestor).await {
                if meta.file_type().is_symlink() {
                    bail!(
                        "Cannot create file under symlink ancestor: {}",
                        ancestor.display()
                    );
                }
            } else {
                break; // 祖先不存在，停止检查
            }
        }
        Ok(target_path)
    } else {
        bail!("Creation path escapes chroot: {}", target_path.display())
    }
}

/// 🔒 为 FTP 提供的带 cwd 的路径验证函数
///
/// # Arguments
/// * `cwd` - 当前工作目录
/// * `home_dir` - 用户主目录（chroot 根目录）
/// * `path` - 要解析的路径（可以是相对或绝对）
///
/// 绝对路径直接验证；相对路径先并入 `cwd` 再验证。
///
/// # Errors
/// 同 [`validate_path_within_chroot`]
pub async fn validate_path_with_cwd(cwd: &str, home_dir: &str, path: &str) -> Result<PathBuf> {
    // 如果是绝对路径，直接验证
    if path.starts_with('/') {
        return validate_path_within_chroot(path, home_dir).await;
    }

    // 如果是相对路径，先连接到 cwd
    let full_path = if path.is_empty() || path == "." {
        cwd.to_string()
    } else {
        format!("{}/{}", cwd.trim_end_matches('/'), path)
    };

    validate_path_within_chroot(&full_path, home_dir).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read as _;

    /// 测试用 `current_thread` runtime（`tokio` 的 `macros` 特性未启用，不能用 `#[tokio::test]`）
    fn block_on<F: std::future::Future>(fut: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(fut)
    }

    fn temp_home() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn home_str(dir: &tempfile::TempDir) -> String {
        dir.path().to_string_lossy().into_owned()
    }

    // ---- 虚拟/真实路径互转 ----

    #[test]
    fn real_to_virtual_path_maps_home_to_root() {
        let dir = temp_home();
        let home = home_str(&dir);
        assert_eq!(real_to_virtual_path(&home, &home), "/");
    }

    #[test]
    fn real_to_virtual_path_strips_home_prefix() {
        let dir = temp_home();
        let home = home_str(&dir);
        std::fs::create_dir_all(dir.path().join("a/b")).unwrap();
        assert_eq!(
            real_to_virtual_path(&dir.path().join("a/b").to_string_lossy(), &home),
            "/a/b"
        );
    }

    #[test]
    fn virtual_to_real_path_root_is_home() {
        let dir = temp_home();
        let home = dir.path().canonicalize().unwrap();
        assert_eq!(
            PathBuf::from(virtual_to_real_path("/", &home_str(&dir))),
            home
        );
    }

    #[test]
    fn virtual_to_real_path_joins_components() {
        let dir = temp_home();
        let home = dir.path().canonicalize().unwrap();
        assert_eq!(
            PathBuf::from(virtual_to_real_path("/a/b.txt", &home_str(&dir))),
            home.join("a/b.txt")
        );
        assert_eq!(
            PathBuf::from(virtual_to_real_path("a/b.txt", &home_str(&dir))),
            home.join("a/b.txt")
        );
    }

    // ---- 用户名合法性 ----

    #[test]
    fn is_safe_username_accepts_alnum_underscore_dash() {
        for name in ["alice", "a", "A_1-b", "_x", "0start", &"a".repeat(64)] {
            assert!(is_safe_username(name), "应接受: {name}");
        }
    }

    #[test]
    fn is_safe_username_rejects_bad_input() {
        for name in ["", "-lead", "has space", "中文", "a;b", "a/b", ".", ".."] {
            assert!(!is_safe_username(name), "应拒绝: {name:?}");
        }
        assert!(!is_safe_username(&"a".repeat(65)), "超过 64 字符应拒绝");
    }

    // ---- safe_resolve_path：chroot 锚定 ----

    #[test]
    fn safe_resolve_path_empty_and_dot_return_home() {
        let dir = temp_home();
        let home = dir.path().canonicalize().unwrap();
        for p in ["", ".", "./", "  "] {
            assert_eq!(safe_resolve_path(&home_str(&dir), p).unwrap(), home);
        }
    }

    #[test]
    fn safe_resolve_path_existing_relative_and_absolute() {
        let dir = temp_home();
        let home = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(dir.path().join("sub")).unwrap();

        assert_eq!(
            safe_resolve_path(&home_str(&dir), "sub").unwrap(),
            home.join("sub")
        );
        assert_eq!(
            safe_resolve_path(&home_str(&dir), "/sub").unwrap(),
            home.join("sub")
        );
    }

    #[test]
    fn safe_resolve_path_nonexistent_stays_inside_home() {
        let dir = temp_home();
        let home = dir.path().canonicalize().unwrap();
        assert_eq!(
            safe_resolve_path(&home_str(&dir), "newdir/file.txt").unwrap(),
            home.join("newdir/file.txt")
        );
    }

    #[test]
    fn safe_resolve_path_blocks_parent_escape() {
        let dir = temp_home();
        std::fs::create_dir_all(dir.path().join("sub")).unwrap();
        let home = home_str(&dir);

        assert!(safe_resolve_path(&home, "..").is_err());
        assert!(safe_resolve_path(&home, "../..").is_err());
        assert!(safe_resolve_path(&home, "sub/../../..").is_err());
    }

    #[test]
    fn safe_resolve_path_blocks_symlink_escape() {
        let dir = temp_home();
        let home = home_str(&dir);
        std::os::unix::fs::symlink("/etc", dir.path().join("etc_link")).unwrap();
        assert!(safe_resolve_path(&home, "etc_link/passwd").is_err());
    }

    #[test]
    fn safe_resolve_path_rejects_overlong_path() {
        let dir = temp_home();
        let long = "a".repeat(5000);
        assert!(matches!(
            safe_resolve_path(&home_str(&dir), &long),
            Err(WftpgError::PathResolveError(_))
        ));
    }

    #[test]
    fn safe_resolve_path_missing_home_fails() {
        assert!(safe_resolve_path("/nonexistent/wftpd/home", "x").is_err());
    }

    // ---- safe_resolve_path_with_cwd ----

    #[test]
    fn safe_resolve_path_with_cwd_resolves_relative_to_cwd() {
        let dir = temp_home();
        let home = dir.path().canonicalize().unwrap();
        let sub = home.join("sub");
        std::fs::create_dir_all(&sub).unwrap();

        assert_eq!(
            safe_resolve_path_with_cwd(&sub.to_string_lossy(), &home_str(&dir), "f.txt").unwrap(),
            sub.join("f.txt")
        );
        // cwd 内的 .. 被钳制回主目录，不视为攻击
        assert_eq!(
            safe_resolve_path_with_cwd(&sub.to_string_lossy(), &home_str(&dir), "../x").unwrap(),
            home.join("x")
        );
    }

    #[test]
    fn safe_resolve_path_with_cwd_rejects_outside_cwd() {
        let dir = temp_home();
        assert!(safe_resolve_path_with_cwd("/etc", &home_str(&dir), "").is_err());
    }

    // ---- MLST/时间/模式格式化 ----

    #[test]
    fn escape_mlst_filename_escapes_specials() {
        assert_eq!(escape_mlst_filename("a b"), "a\\ b");
        assert_eq!(escape_mlst_filename("x;y=z"), "x\\;y\\=z");
        assert_eq!(escape_mlst_filename("a\\b"), "a\\\\b");
        assert_eq!(escape_mlst_filename("\n"), "\\012");
        assert_eq!(escape_mlst_filename("\r"), "\\015");
        assert_eq!(escape_mlst_filename("\t"), "\\011");
        assert_eq!(escape_mlst_filename("\u{1}"), "\\001");
        assert_eq!(escape_mlst_filename("plain.txt"), "plain.txt");
    }

    #[test]
    fn file_metadata_formatting() {
        let dir = temp_home();
        let file = dir.path().join("f.txt");
        std::fs::write(&file, b"hello").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        let md = std::fs::metadata(&file).unwrap();

        let mtime = get_file_mtime(&md);
        assert_eq!(mtime.len(), 16, "格式应为 YYYY-MM-DD HH:MM");
        assert!(mtime.contains('-'));

        let raw = get_file_mtime_raw(&md);
        assert!(raw.parse::<u64>().is_ok());

        #[cfg(unix)]
        assert_eq!(get_unix_mode(&md), "0644");

        let facts = build_mlst_facts(&md);
        assert!(facts.starts_with("type=file;"));
        assert!(facts.contains("size=5;"));
        assert!(facts.contains("unix.mode="));

        let dir_md = std::fs::metadata(dir.path()).unwrap();
        assert!(build_mlst_facts(&dir_md).starts_with("type=dir;"));
    }

    // ---- safe_open_file_at：openat(O_NOFOLLOW) 防符号链接 ----

    #[test]
    fn safe_open_file_at_reads_file_inside_home() {
        let dir = temp_home();
        std::fs::write(dir.path().join("data.txt"), b"hello").unwrap();

        let mut f = safe_open_file_at(&home_str(&dir), "data.txt").unwrap();
        let mut content = String::new();
        f.read_to_string(&mut content).unwrap();
        assert_eq!(content, "hello");
    }

    #[test]
    fn safe_open_file_at_opens_nested_components() {
        let dir = temp_home();
        std::fs::create_dir_all(dir.path().join("a/b")).unwrap();
        std::fs::write(dir.path().join("a/b/c.txt"), b"deep").unwrap();

        let mut f = safe_open_file_at(&home_str(&dir), "a/b/c.txt").unwrap();
        let mut content = String::new();
        f.read_to_string(&mut content).unwrap();
        assert_eq!(content, "deep");
    }

    #[test]
    fn safe_open_file_at_rejects_parent_dir() {
        let dir = temp_home();
        assert!(safe_open_file_at(&home_str(&dir), "../secret").is_err());
        assert!(safe_open_file_at(&home_str(&dir), "a/../b").is_err());
    }

    #[test]
    fn safe_open_file_at_rejects_symlink() {
        let dir = temp_home();
        std::fs::write(dir.path().join("real.txt"), b"r").unwrap();
        std::os::unix::fs::symlink("real.txt", dir.path().join("ln.txt")).unwrap();
        assert!(safe_open_file_at(&home_str(&dir), "ln.txt").is_err());
    }

    #[test]
    fn safe_open_file_at_rejects_absolute_escape() {
        let dir = temp_home();
        assert!(safe_open_file_at(&home_str(&dir), "/etc/passwd").is_err());
    }

    // ---- 异步变体 ----

    #[test]
    fn safe_resolve_path_async_basic_and_escape() {
        let dir = temp_home();
        let home = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(dir.path().join("sub")).unwrap();
        std::os::unix::fs::symlink("/etc", dir.path().join("etc_link")).unwrap();
        let hs = home_str(&dir);

        assert_eq!(
            block_on(safe_resolve_path_async(&hs, "sub")).unwrap(),
            home.join("sub")
        );
        assert!(block_on(safe_resolve_path_async(&hs, "..")).is_err());
        assert!(block_on(safe_resolve_path_async(&hs, "etc_link/passwd")).is_err());
    }

    #[test]
    fn safe_open_file_at_async_reads_file() {
        let dir = temp_home();
        std::fs::write(dir.path().join("data.txt"), b"async").unwrap();
        let mut f = block_on(safe_open_file_at_async(&home_str(&dir), "data.txt")).unwrap();
        let content = block_on(async {
            use tokio::io::AsyncReadExt as _;
            let mut buf = Vec::new();
            f.read_to_end(&mut buf).await.unwrap();
            buf
        });
        assert_eq!(content, b"async");
    }

    #[test]
    fn validate_path_within_chroot_ok_and_escape() {
        let dir = temp_home();
        let home = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(dir.path().join("sub")).unwrap();
        std::os::unix::fs::symlink("/etc", dir.path().join("etc_link")).unwrap();
        let hs = home_str(&dir);

        assert_eq!(
            block_on(validate_path_within_chroot("sub/f.txt", &hs)).unwrap(),
            home.join("sub/f.txt")
        );
        // chroot 语义：绝对路径视为主目录内的路径
        assert_eq!(
            block_on(validate_path_within_chroot("/etc/passwd", &hs)).unwrap(),
            home.join("etc/passwd")
        );
        assert!(block_on(validate_path_within_chroot("etc_link/passwd", &hs)).is_err());
    }

    #[test]
    fn validate_path_for_creation_rules() {
        let dir = temp_home();
        let home = dir.path().canonicalize().unwrap();
        let hs = home_str(&dir);

        assert_eq!(
            block_on(validate_path_for_creation("newd/x.txt", &hs)).unwrap(),
            home.join("newd/x.txt")
        );
        // 根目录不可创建
        assert!(block_on(validate_path_for_creation("/", &hs)).is_err());
    }

    #[test]
    fn validate_path_with_cwd_joins_relative() {
        let dir = temp_home();
        let home = dir.path().canonicalize().unwrap();
        // sub 必须真实存在：canonicalize 才能解析 ".." 并发现逃逸
        std::fs::create_dir_all(dir.path().join("sub")).unwrap();
        let hs = home_str(&dir);

        assert_eq!(
            block_on(validate_path_with_cwd("/sub", &hs, "f.txt")).unwrap(),
            home.join("sub/f.txt")
        );
        // 空路径/`.` 解析为 cwd 本身
        assert_eq!(
            block_on(validate_path_with_cwd("/sub", &hs, ".")).unwrap(),
            home.join("sub")
        );
        // cwd 之上的路径拒绝
        assert!(block_on(validate_path_with_cwd("/sub", &hs, "../../x")).is_err());
    }
}
