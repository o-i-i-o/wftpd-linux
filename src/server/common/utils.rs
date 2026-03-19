use std::path::{Path, PathBuf};

use crate::core::error::{WftpgError, WftpgResult};

const MAX_PATH_LENGTH: usize = 4096;

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
    
    if !home.exists() {
        return Err(WftpgError::PathResolveError(
            format!("主目录不存在: {:?}", home)
        ));
    }
    
    match home.canonicalize() {
        Ok(canon) => Ok(canon),
        Err(e) => {
            Err(WftpgError::PathResolveError(
                format!("无法规范化主目录 {:?}: {}", home, e)
            ))
        }
    }
}

fn resolve_existing_path(resolved: &Path, home_canon: &Path) -> WftpgResult<PathBuf> {
    match resolved.canonicalize() {
        Ok(canon) => {
            if canon.starts_with(home_canon) {
                Ok(canon)
            } else {
                Err(WftpgError::PathResolveError(
                    format!("路径遍历攻击被阻止: {:?} 位于主目录 {:?} 之外", resolved, home_canon)
                ))
            }
        }
        Err(e) => {
            Err(WftpgError::PathResolveError(
                format!("无法规范化路径 {:?}: {}", resolved, e)
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
                if safe_path.starts_with(home_canon) && safe_path != home_canon {
                    if !safe_path.pop() {
                        return Err(WftpgError::PathResolveError(
                            format!("路径遍历攻击: {:?} 中包含过多的父目录", original_path)
                        ));
                    }
                } else {
                    return Err(WftpgError::PathResolveError(
                        format!("路径遍历攻击被阻止: {:?} 中无法访问主目录之上的目录", original_path)
                    ));
                }
            }
            std::path::Component::CurDir => {}
            _ => {}
        }
    }
    
    if safe_path.starts_with(home_canon) {
        Ok(safe_path)
    } else {
        Err(WftpgError::PathResolveError(
            format!("路径遍历攻击被阻止: {:?} 逃逸了主目录 {:?}", safe_path, home_canon)
        ))
    }
}

fn resolve_nonexistent_path(resolved: &Path, home_canon: &Path, original_path: &str) -> WftpgResult<PathBuf> {
    let clean_path = original_path.trim();
    
    if clean_path.starts_with('/') {
        if resolved.starts_with(home_canon) {
            Ok(resolved.to_path_buf())
        } else {
            Err(WftpgError::PathResolveError(
                format!("绝对路径位于主目录之外: {:?}", resolved)
            ))
        }
    } else {
        apply_path_components_safe(home_canon.to_path_buf(), home_canon, clean_path, original_path)
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

    let resolved = if clean_path.starts_with('/') {
        PathBuf::from(clean_path)
    } else {
        home_canon.join(clean_path)
    };

    if resolved.exists() {
        resolve_existing_path(&resolved, &home_canon)
    } else {
        resolve_nonexistent_path(&resolved, &home_canon, path)
    }
}

pub fn safe_resolve_path_with_cwd(cwd: &str, home_dir: &str, path: &str) -> WftpgResult<PathBuf> {
    check_path_length(path)?;

    let home_canon = canonicalize_home(home_dir)?;
    
    let clean_path = path.trim();
    
    if clean_path.is_empty() || clean_path == "." || clean_path == "./" {
        return resolve_cwd(cwd, &home_canon);
    }
    
    let resolved = if clean_path.starts_with('/') {
        PathBuf::from(clean_path)
    } else {
        Path::new(cwd).join(clean_path)
    };
    
    if resolved.exists() {
        resolve_existing_path(&resolved, &home_canon)
    } else {
        resolve_nonexistent_path_with_cwd(&resolved, &home_canon, cwd, path)
    }
}

fn resolve_nonexistent_path_with_cwd(resolved: &Path, home_canon: &Path, cwd: &str, original_path: &str) -> WftpgResult<PathBuf> {
    let clean_path = original_path.trim();
    
    if clean_path.starts_with('/') {
        if resolved.starts_with(home_canon) {
            Ok(resolved.to_path_buf())
        } else {
            Err(WftpgError::PathResolveError(
                format!("绝对路径位于主目录之外: {:?}", resolved)
            ))
        }
    } else {
        let cwd_canon = resolve_cwd(cwd, home_canon)?;
        apply_path_components_safe(cwd_canon, home_canon, clean_path, original_path)
    }
}

fn resolve_cwd(cwd: &str, home_canon: &Path) -> WftpgResult<PathBuf> {
    if cwd.is_empty() {
        return Ok(home_canon.to_path_buf());
    }
    
    let cwd_path = PathBuf::from(cwd);
    if cwd_path.exists() {
        match cwd_path.canonicalize() {
            Ok(canon) => {
                if canon.starts_with(home_canon) {
                    Ok(canon)
                } else {
                    Err(WftpgError::PathResolveError(
                        format!("当前工作目录位于主目录之外: {:?}", cwd_path)
                    ))
                }
            }
            Err(e) => {
                Err(WftpgError::PathResolveError(
                    format!("无法规范化当前工作目录 {:?}: {}", cwd_path, e)
                ))
            }
        }
    } else {
        Err(WftpgError::PathResolveError(
            format!("当前工作目录不存在或无法访问: {:?}", cwd_path)
        ))
    }
}

pub fn get_file_mtime(metadata: &std::fs::Metadata) -> String {
    use std::time::UNIX_EPOCH;
    if let Ok(time) = metadata.modified() {
        if let Ok(duration) = time.duration_since(UNIX_EPOCH) {
            let secs = duration.as_secs();
            let datetime = chrono::DateTime::from_timestamp(secs as i64, 0);
            if let Some(dt) = datetime {
                return dt.format("%Y-%m-%d %H:%M").to_string();
            }
        }
    }
    "1970-01-01 00:00".to_string()
}

pub fn get_file_mtime_raw(metadata: &std::fs::Metadata) -> String {
    use std::time::UNIX_EPOCH;
    if let Ok(time) = metadata.modified() {
        if let Ok(duration) = time.duration_since(UNIX_EPOCH) {
            return format!("{}", duration.as_secs());
        }
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
                result.push_str(&format!("\\{:03o}", c as u8));
            }
            c => result.push(c),
        }
    }
    result
}

pub fn format_mtime_rfc3659(metadata: &std::fs::Metadata) -> String {
    use std::time::UNIX_EPOCH;
    if let Ok(time) = metadata.modified() {
        if let Ok(duration) = time.duration_since(UNIX_EPOCH) {
            let secs = duration.as_secs();
            let nanos = duration.subsec_nanos();
            if let Some(dt) = chrono::DateTime::from_timestamp(secs as i64, nanos) {
                let formatted = dt.format("%Y%m%d%H%M%S%.3f").to_string();
                return formatted;
            }
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
