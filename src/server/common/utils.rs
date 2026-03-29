use std::path::{Path, PathBuf};
use tracing::{info, debug, warn, error};

use crate::core::error::{WftpgError, WftpgResult};

const MAX_PATH_LENGTH: usize = 4096;

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

pub fn is_safe_username(username: &str) -> bool {
    if username.is_empty() || username.len() > 64 {
        return false;
    }
    let bytes = username.as_bytes();
    let first = bytes[0];
    if !first.is_ascii_alphanumeric() && first != b'_' {
        return false;
    }
    bytes.iter().all(|&c| {
        c.is_ascii_alphanumeric() || c == b'_' || c == b'-'
    })
}

fn check_path_length(path: &str) -> WftpgResult<()> {
    if path.len() > MAX_PATH_LENGTH {
        return Err(WftpgError::PathResolveError(
            format!("路径过长: {} 字节 (最大 {})", path.len(), MAX_PATH_LENGTH)
        ));
    }
    Ok(())
}

fn canonicalize_home(home_dir: &str) -> WftpgResult<PathBuf> {
    let home = PathBuf::from(home_dir);
    
    match home.canonicalize() {
        Ok(canon) => Ok(canon),
        Err(e) => {
            error!(home = ?home, error = %e, "无法规范化主目录");
            Err(WftpgError::PathResolveError(
                format!("主目录不存在或无法访问：{:?}", home)
            ))
        }
    }
}

async fn canonicalize_home_async(home_dir: &str) -> WftpgResult<PathBuf> {
    let home = PathBuf::from(home_dir);
    
    match tokio::fs::canonicalize(&home).await {
        Ok(canon) => Ok(canon),
        Err(e) => {
            error!(home = ?home, error = %e, "无法规范化主目录");
            Err(WftpgError::PathResolveError(
                format!("主目录不存在或无法访问：{:?}", home)
            ))
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
                            "路径中包含过多的父目录".to_string()
                        ));
                    }
                } else {
                    warn!("路径遍历攻击被阻止: {:?} 中无法访问主目录之上的目录", original_path);
                    return Err(WftpgError::PathResolveError(
                        "无法访问主目录之上的目录".to_string()
                    ));
                }
            }
            std::path::Component::CurDir => {}
            std::path::Component::RootDir => {
                if clean_path.starts_with('/') {
                    safe_path = home_canon.to_path_buf();
                }
            }
            std::path::Component::Prefix(_) => {}
        }
    }
    
    if safe_path.starts_with(home_canon) {
        if safe_path.exists() {
            match safe_path.canonicalize() {
                Ok(canon) => {
                    if canon.starts_with(home_canon) {
                        Ok(canon)
                    } else {
                        warn!("路径遍历攻击被阻止: {:?} 逃逸了主目录 {:?}", canon, home_canon);
                        Err(WftpgError::PathResolveError(
                            "路径逃逸了主目录".to_string()
                        ))
                    }
                }
                Err(e) => {
                    warn!("无法规范化路径 {:?}: {}", safe_path, e);
                    Err(WftpgError::PathResolveError(
                        "无法访问路径".to_string()
                    ))
                }
            }
        } else {
            Ok(safe_path)
        }
    } else {
        warn!("路径遍历攻击被阻止: {:?} 逃逸了主目录 {:?}", safe_path, home_canon);
        Err(WftpgError::PathResolveError(
            "路径逃逸了主目录".to_string()
        ))
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
                            "路径中包含过多的父目录".to_string()
                        ));
                    }
                } else {
                    warn!("路径遍历攻击被阻止: {:?} 中无法访问主目录之上的目录", original_path);
                    return Err(WftpgError::PathResolveError(
                        "无法访问主目录之上的目录".to_string()
                    ));
                }
            }
            std::path::Component::CurDir => {}
            std::path::Component::RootDir => {
                if clean_path.starts_with('/') {
                    safe_path = home_canon.to_path_buf();
                }
            }
            std::path::Component::Prefix(_) => {}
        }
    }
    
    if safe_path.starts_with(home_canon) {
        match tokio::fs::try_exists(&safe_path).await {
            Ok(true) => {
                match tokio::fs::canonicalize(&safe_path).await {
                    Ok(canon) => {
                        if canon.starts_with(home_canon) {
                            Ok(canon)
                        } else {
                            warn!("路径遍历攻击被阻止: {:?} 逃逸了主目录 {:?}", canon, home_canon);
                            Err(WftpgError::PathResolveError(
                                "路径逃逸了主目录".to_string()
                            ))
                        }
                    }
                    Err(e) => {
                        warn!("无法规范化路径 {:?}: {}", safe_path, e);
                        Err(WftpgError::PathResolveError(
                            "无法访问路径".to_string()
                        ))
                    }
                }
            }
            Ok(false) => Ok(safe_path),
            Err(e) => {
                warn!("无法检查路径是否存在 {:?}: {}", safe_path, e);
                Err(WftpgError::PathResolveError(
                    "无法访问路径".to_string()
                ))
            }
        }
    } else {
        warn!("路径遍历攻击被阻止: {:?} 逃逸了主目录 {:?}", safe_path, home_canon);
        Err(WftpgError::PathResolveError(
            "路径逃逸了主目录".to_string()
        ))
    }
}

fn resolve_nonexistent_path(resolved: &Path, home_canon: &Path, original_path: &str) -> WftpgResult<PathBuf> {
    let clean_path = original_path.trim();
    
    if Path::new(clean_path).is_absolute() {
        if resolved.starts_with(home_canon) {
            Ok(resolved.to_path_buf())
        } else {
            warn!("绝对路径位于主目录之外: {:?}", resolved);
            Err(WftpgError::PathResolveError(
                "绝对路径位于主目录之外".to_string()
            ))
        }
    } else {
        apply_path_components_safe(home_canon.to_path_buf(), home_canon, clean_path, original_path)
    }
}

async fn resolve_nonexistent_path_async(resolved: &Path, home_canon: &Path, original_path: &str) -> WftpgResult<PathBuf> {
    let clean_path = original_path.trim();
    
    if Path::new(clean_path).is_absolute() {
        if resolved.starts_with(home_canon) {
            Ok(resolved.to_path_buf())
        } else {
            warn!("绝对路径位于主目录之外: {:?}", resolved);
            Err(WftpgError::PathResolveError(
                "绝对路径位于主目录之外".to_string()
            ))
        }
    } else {
        apply_path_components_safe_async(home_canon.to_path_buf(), home_canon, clean_path, original_path).await
    }
}

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
                warn!("路径遍历攻击被阻止: {:?} 位于主目录 {:?} 之外", canon, home_canon);
                Err(WftpgError::PathResolveError(
                    "路径位于主目录之外".to_string()
                ))
            }
        }
        Err(_) => {
            resolve_nonexistent_path(&resolved, &home_canon, path)
        }
    }
}

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
                warn!("路径遍历攻击被阻止: {:?} 位于主目录 {:?} 之外", canon, home_canon);
                Err(WftpgError::PathResolveError(
                    "路径位于主目录之外".to_string()
                ))
            }
        }
        Err(_) => {
            resolve_nonexistent_path_async(&resolved, &home_canon, path).await
        }
    }
}

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
                warn!("路径遍历攻击被阻止: {:?} 位于主目录 {:?} 之外", canon, home_canon);
                Err(WftpgError::PathResolveError(
                    "路径位于主目录之外".to_string()
                ))
            }
        }
        Err(_) => {
            resolve_nonexistent_path_with_cwd(&resolved, &home_canon, cwd, path)
        }
    }
}

pub async fn safe_resolve_path_with_cwd_async(cwd: &str, home_dir: &str, path: &str) -> WftpgResult<PathBuf> {
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
                warn!("路径遍历攻击被阻止: {:?} 位于主目录 {:?} 之外", canon, home_canon);
                Err(WftpgError::PathResolveError(
                    "路径位于主目录之外".to_string()
                ))
            }
        }
        Err(_) => {
            resolve_nonexistent_path_with_cwd_async(&resolved, &home_canon, cwd, path).await
        }
    }
}

fn resolve_nonexistent_path_with_cwd(resolved: &Path, home_canon: &Path, cwd: &str, original_path: &str) -> WftpgResult<PathBuf> {
    let clean_path = original_path.trim();
    
    if Path::new(clean_path).is_absolute() {
        if resolved.starts_with(home_canon) {
            Ok(resolved.to_path_buf())
        } else {
            warn!("绝对路径位于主目录之外: {:?}", resolved);
            Err(WftpgError::PathResolveError(
                "绝对路径位于主目录之外".to_string()
            ))
        }
    } else {
        let cwd_canon = resolve_cwd(cwd, home_canon)?;
        apply_path_components_safe(cwd_canon, home_canon, clean_path, original_path)
    }
}

async fn resolve_nonexistent_path_with_cwd_async(resolved: &Path, home_canon: &Path, cwd: &str, original_path: &str) -> WftpgResult<PathBuf> {
    let clean_path = original_path.trim();
    
    if Path::new(clean_path).is_absolute() {
        if resolved.starts_with(home_canon) {
            Ok(resolved.to_path_buf())
        } else {
            warn!("绝对路径位于主目录之外: {:?}", resolved);
            Err(WftpgError::PathResolveError(
                "绝对路径位于主目录之外".to_string()
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
                    "当前工作目录位于主目录之外".to_string()
                ))
            }
        }
        Err(e) => {
            warn!("当前工作目录不存在或无法访问 {:?}: {}", cwd_path, e);
            Err(WftpgError::PathResolveError(
                "当前工作目录不存在或无法访问".to_string()
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
                    "当前工作目录位于主目录之外".to_string()
                ))
            }
        }
        Err(e) => {
            warn!("当前工作目录不存在或无法访问 {:?}: {}", cwd_path, e);
            Err(WftpgError::PathResolveError(
                "当前工作目录不存在或无法访问".to_string()
            ))
        }
    }
}

pub fn get_file_mtime(metadata: &std::fs::Metadata) -> String {
    use chrono::DateTime;
    use std::time::UNIX_EPOCH;
    
    if let Ok(system_time) = metadata.modified()
        && let Ok(duration) = system_time.duration_since(UNIX_EPOCH) {
            let secs = duration.as_secs() as i64;
            let nanos = duration.subsec_nanos();
            if let Some(dt) = DateTime::from_timestamp(secs, nanos) {
                return dt.format("%Y-%m-%d %H:%M").to_string();
            }
        }
    "1970-01-01 00:00".to_string()
}

pub fn get_file_mtime_raw(metadata: &std::fs::Metadata) -> String {
    use std::time::UNIX_EPOCH;
    if let Ok(time) = metadata.modified()
        && let Ok(duration) = time.duration_since(UNIX_EPOCH) {
            return format!("{}", duration.as_secs());
        }
    "0".to_string()
}

pub fn escape_mlst_filename(name: &str) -> String {
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
                let code = c as u32;
                if code <= 0xFF {
                    result.push_str(&format!("\\{:03o}", code as u8));
                } else {
                    result.push('?');
                }
            }
            c => result.push(c),
        }
    }
    result
}

pub fn format_mtime_rfc3659(metadata: &std::fs::Metadata) -> String {
    use chrono::DateTime;
    use std::time::UNIX_EPOCH;
    
    if let Ok(system_time) = metadata.modified()
        && let Ok(duration) = system_time.duration_since(UNIX_EPOCH) {
            let secs = duration.as_secs() as i64;
            let nanos = duration.subsec_nanos();
            if let Some(dt) = DateTime::from_timestamp(secs, nanos) {
                return dt.format("%Y%m%d%H%M%S%.3f").to_string();
            }
        }
    "19700101000000".to_string()
}

pub fn get_unix_mode(metadata: &std::fs::Metadata) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = metadata.permissions().mode();
        format!("{:04o}", mode & 0o7777)
    }
    #[cfg(not(unix))]
    {
        if metadata.is_dir() {
            "0755".to_string()
        } else {
            "0644".to_string()
        }
    }
}

pub fn build_mlst_facts(metadata: &std::fs::Metadata) -> String {
    let mut facts: Vec<String> = Vec::new();

    if metadata.is_dir() {
        facts.push("type=dir;".to_string());
    } else {
        facts.push("type=file;".to_string());
    }

    facts.push(format!("size={};", metadata.len()));

    let mtime = format_mtime_rfc3659(metadata);
    facts.push(format!("modify={};", mtime));

    let mode = get_unix_mode(metadata);
    facts.push(format!("unix.mode={};", mode));

    facts.join("")
}

#[cfg(unix)]
pub fn safe_open_file_at(home_dir: &str, relative_path: &str) -> WftpgResult<std::fs::File> {
    use nix::fcntl::{openat, OFlag, AT_FDCWD};
    use nix::sys::stat::Mode;
    use std::os::fd::AsFd;
    
    let home_canon = canonicalize_home(home_dir)?;
    
    let dirfd = openat(
        AT_FDCWD,
        &home_canon,
        OFlag::O_DIRECTORY | OFlag::O_RDONLY,
        Mode::empty()
    ).map_err(|e| {
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
                "路径中不允许包含父目录引用".to_string()
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
                return Err(WftpgError::PathResolveError(
                    format!("无法访问路径组件: {}", component)
                ));
            }
        }
    }
    
    for fd in fds_to_close {
        nix::unistd::close(fd).ok();
    }
    
    Ok(std::fs::File::from(current_fd))
}

#[cfg(unix)]
pub async fn safe_open_file_at_async(home_dir: &str, relative_path: &str) -> WftpgResult<tokio::fs::File> {
    use nix::fcntl::{openat, OFlag, AT_FDCWD};
    use nix::sys::stat::Mode;
    use std::os::fd::AsFd;
    
    let home_canon = canonicalize_home_async(home_dir).await?;
    
    let dirfd = openat(
        AT_FDCWD,
        &home_canon,
        OFlag::O_DIRECTORY | OFlag::O_RDONLY,
        Mode::empty()
    ).map_err(|e| {
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
                "路径中不允许包含父目录引用".to_string()
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
                return Err(WftpgError::PathResolveError(
                    format!("无法访问路径组件: {}", component)
                ));
            }
        }
    }
    
    for fd in fds_to_close {
        nix::unistd::close(fd).ok();
    }
    
    let std_file = std::fs::File::from(current_fd);
    Ok(tokio::fs::File::from_std(std_file))
}
