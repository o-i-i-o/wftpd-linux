use std::path::{Path, PathBuf};

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

fn is_path_too_long(path: &str) -> bool {
    if path.len() > MAX_PATH_LENGTH {
        log::warn!("Path too long: {} bytes (max {})", path.len(), MAX_PATH_LENGTH);
        true
    } else {
        false
    }
}

fn canonicalize_home(home_dir: &str) -> Option<PathBuf> {
    let home = PathBuf::from(home_dir);
    
    if !home.exists() {
        log::warn!("Home directory does not exist: {:?}", home);
        return None;
    }
    
    match home.canonicalize() {
        Ok(canon) => Some(canon),
        Err(e) => {
            log::warn!("Failed to canonicalize home directory {:?}: {}", home, e);
            if home.exists() {
                Some(home)
            } else {
                None
            }
        }
    }
}

fn resolve_existing_path(resolved: &Path, home_canon: &Path) -> PathBuf {
    match resolved.canonicalize() {
        Ok(canon) if canon.starts_with(home_canon) => canon,
        Ok(_) => {
            log::warn!("Path traversal attempt blocked: {:?} is outside home {:?}", resolved, home_canon);
            home_canon.to_path_buf()
        }
        Err(e) => {
            log::warn!("Failed to canonicalize path {:?}: {}", resolved, e);
            home_canon.to_path_buf()
        }
    }
}

fn apply_path_components_safe(
    base: PathBuf,
    home_canon: &Path,
    clean_path: &str,
    original_path: &str,
) -> PathBuf {
    let mut safe_path = base;
    
    for component in Path::new(clean_path).components() {
        match component {
            std::path::Component::Normal(name) => {
                safe_path.push(name);
            }
            std::path::Component::ParentDir => {
                if safe_path.starts_with(home_canon) && safe_path != home_canon {
                    if !safe_path.pop() {
                        log::warn!("Path traversal attempt: too many parent directories in {:?}", original_path);
                        return home_canon.to_path_buf();
                    }
                } else {
                    log::warn!("Path traversal attempt blocked: cannot go above home in {:?}", original_path);
                    return home_canon.to_path_buf();
                }
            }
            std::path::Component::CurDir => {}
            _ => {}
        }
    }
    
    if safe_path.starts_with(home_canon) {
        safe_path
    } else {
        log::warn!("Path traversal attempt blocked: {:?} escaped home {:?}", safe_path, home_canon);
        home_canon.to_path_buf()
    }
}

fn resolve_nonexistent_path(resolved: &Path, home_canon: &Path, original_path: &str) -> PathBuf {
    let clean_path = original_path.trim();
    
    if clean_path.starts_with('/') {
        if resolved.starts_with(home_canon) {
            resolved.to_path_buf()
        } else {
            log::warn!("Absolute path outside home directory: {:?}", resolved);
            home_canon.to_path_buf()
        }
    } else {
        apply_path_components_safe(home_canon.to_path_buf(), home_canon, clean_path, original_path)
    }
}

pub fn safe_resolve_path(home_dir: &str, path: &str) -> PathBuf {
    if is_path_too_long(path) {
        return PathBuf::from(home_dir);
    }

    let home_canon = match canonicalize_home(home_dir) {
        Some(c) => c,
        None => return PathBuf::from(home_dir),
    };
    
    let clean_path = path.trim();

    if clean_path.is_empty() || clean_path == "." || clean_path == "./" {
        return home_canon;
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

pub fn safe_resolve_path_with_cwd(cwd: &str, home_dir: &str, path: &str) -> PathBuf {
    if is_path_too_long(path) {
        return PathBuf::from(home_dir);
    }

    let home_canon = match canonicalize_home(home_dir) {
        Some(c) => c,
        None => return PathBuf::from(home_dir),
    };
    
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

fn resolve_nonexistent_path_with_cwd(resolved: &Path, home_canon: &Path, cwd: &str, original_path: &str) -> PathBuf {
    let clean_path = original_path.trim();
    
    if clean_path.starts_with('/') {
        if resolved.starts_with(home_canon) {
            resolved.to_path_buf()
        } else {
            log::warn!("Absolute path outside home directory: {:?}", resolved);
            home_canon.to_path_buf()
        }
    } else {
        let cwd_canon = resolve_cwd(cwd, home_canon);
        apply_path_components_safe(cwd_canon, home_canon, clean_path, original_path)
    }
}

fn resolve_cwd(cwd: &str, home_canon: &Path) -> PathBuf {
    let cwd_path = PathBuf::from(cwd);
    if cwd_path.exists() {
        match cwd_path.canonicalize() {
            Ok(canon) if canon.starts_with(home_canon) => canon,
            Ok(_) => {
                log::warn!("CWD outside home directory: {:?}", cwd_path);
                home_canon.to_path_buf()
            }
            Err(e) => {
                log::warn!("Failed to canonicalize CWD {:?}: {}", cwd_path, e);
                home_canon.to_path_buf()
            }
        }
    } else {
        log::warn!("CWD does not exist or is inaccessible: {:?}", cwd_path);
        home_canon.to_path_buf()
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
