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
            _ => home_canon,
        }
    } else {
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
            _ => home_canon,
        }
    } else {
        let mut safe_path = PathBuf::from(cwd);
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
            let days = secs / 86400;
            let years = 1970 + days / 365;
            let remaining_days = days % 365;
            let months = remaining_days / 30 + 1;
            let day = remaining_days % 30 + 1;
            let hour = (secs % 86400) / 3600;
            let minute = (secs % 3600) / 60;
            return format!("{:04}-{:02}-{:02} {:02}:{:02}", years, months, day, hour, minute);
        }
    }
    "Jan 01 00:00".to_string()
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

pub fn build_mlst_facts(metadata: &std::fs::Metadata) -> String {
    use std::time::UNIX_EPOCH;
    let mut facts: Vec<String> = Vec::new();

    if metadata.is_dir() {
        facts.push("type=dir;".to_string());
    } else {
        facts.push("type=file;".to_string());
    }

    facts.push(format!("size={};", metadata.len()));

    if let Ok(time) = metadata.modified() {
        if let Ok(duration) = time.duration_since(UNIX_EPOCH) {
            facts.push(format!("modify={};", duration.as_secs()));
        }
    }

    facts.join("")
}
