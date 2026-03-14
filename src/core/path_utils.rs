use std::path::{Path, PathBuf};

pub fn safe_resolve_path(home_dir: &str, base_path: &str, path: &str) -> PathBuf {
    let home = PathBuf::from(home_dir);
    let clean_path = path.trim();
    
    if clean_path.is_empty() || clean_path == "." || clean_path == "./" {
        return PathBuf::from(base_path);
    }
    
    let resolved = if clean_path.starts_with('/') {
        PathBuf::from(clean_path)
    } else {
        Path::new(base_path).join(clean_path)
    };
    
    if resolved.exists() {
        match resolved.canonicalize() {
            Ok(canon) if canon.starts_with(&home) => canon,
            _ => home,
        }
    } else {
        let mut safe_path = home.clone();
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
        if safe_path.starts_with(&home) {
            safe_path
        } else {
            home
        }
    }
}

pub fn safe_resolve_path_with_home(home_dir: &str, path: &str) -> PathBuf {
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
