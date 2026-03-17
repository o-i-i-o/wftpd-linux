use std::path::PathBuf;

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

pub fn safe_resolve_path(home_dir: &str, path: &str) -> PathBuf {
    let home = PathBuf::from(home_dir);
    
    if !home.exists() {
        return home;
    }
    
    let home_canon = match home.canonicalize() {
        Ok(c) => c,
        Err(_) => home.clone(),
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
        match resolved.canonicalize() {
            Ok(canon) if canon.starts_with(&home_canon) => canon,
            Ok(_) => home_canon,
            _ => home_canon,
        }
    } else {
        if clean_path.starts_with('/') {
            if resolved.starts_with(&home_canon) {
                return resolved;
            } else {
                return home_canon;
            }
        }
        
        let mut safe_path = home_canon.clone();
        for component in resolved.components() {
            match component {
                std::path::Component::Normal(name) => {
                    safe_path.push(name);
                }
                std::path::Component::ParentDir => {
                    safe_path.pop();
                }
                _ => {}
            }
        }
        if safe_path.starts_with(&home_canon) {
            safe_path
        } else {
            home_canon
        }
    }
}

pub fn safe_resolve_path_with_cwd(cwd: &str, home_dir: &str, path: &str) -> PathBuf {
    let home = PathBuf::from(home_dir);
    let home_canon = match home.canonicalize() {
        Ok(c) => c,
        Err(_) => {
            if home.exists() {
                home.clone()
            } else {
                return home;
            }
        }
    };
    
    let clean_path = path.trim();
    
    if clean_path.is_empty() || clean_path == "." || clean_path == "./" {
        let cwd_path = PathBuf::from(cwd);
        if cwd_path.exists() {
            match cwd_path.canonicalize() {
                Ok(canon) if canon.starts_with(&home_canon) => return canon,
                _ => return home_canon,
            }
        }
        return cwd_path;
    }
    
    let resolved = if clean_path.starts_with('/') {
        PathBuf::from(clean_path)
    } else {
        std::path::Path::new(cwd).join(clean_path)
    };
    
    if resolved.exists() {
        match resolved.canonicalize() {
            Ok(canon) if canon.starts_with(&home_canon) => canon,
            Ok(_) => home_canon,
            _ => home_canon,
        }
    } else {
        if clean_path.starts_with('/') {
            if resolved.starts_with(&home_canon) {
                return resolved;
            } else {
                return home_canon;
            }
        }
        
        let cwd_path = PathBuf::from(cwd);
        let mut safe_path = if cwd_path.exists() {
            match cwd_path.canonicalize() {
                Ok(canon) if canon.starts_with(&home_canon) => canon,
                _ => home_canon.clone(),
            }
        } else {
            home_canon.clone()
        };
        
        for component in resolved.components() {
            match component {
                std::path::Component::Normal(name) => {
                    safe_path.push(name);
                }
                std::path::Component::ParentDir => {
                    safe_path.pop();
                }
                _ => {}
            }
        }
        if safe_path.starts_with(&home_canon) {
            safe_path
        } else {
            home_canon
        }
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
