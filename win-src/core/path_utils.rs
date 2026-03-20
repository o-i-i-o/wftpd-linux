use std::path::{Component, Path, PathBuf};

const MAX_PATH_DEPTH: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathResolveError {
    PathEscape,
    NotADirectory,
    NotFound,
    PathTooDeep,
    HomeDirectoryNotFound,
    CanonicalizeFailed,
    InvalidPath,
    SymlinkNotAllowed,
    PathNotUnderHome,
}

impl std::fmt::Display for PathResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PathResolveError::PathEscape => write!(f, "路径越界访问"),
            PathResolveError::NotADirectory => write!(f, "路径不是目录"),
            PathResolveError::NotFound => write!(f, "路径不存在"),
            PathResolveError::PathTooDeep => write!(f, "路径深度超过最大限制"),
            PathResolveError::HomeDirectoryNotFound => write!(f, "主目录不存在"),
            PathResolveError::CanonicalizeFailed => write!(f, "路径规范化失败"),
            PathResolveError::InvalidPath => write!(f, "无效路径"),
            PathResolveError::SymlinkNotAllowed => write!(f, "不允许符号链接"),
            PathResolveError::PathNotUnderHome => write!(f, "路径不在主目录下"),
        }
    }
}

impl std::error::Error for PathResolveError {}

fn normalize_windows_path(path: &Path) -> PathBuf {
    let path_str = path.to_string_lossy();
    let stripped = path_str.strip_prefix(r"\\?\").unwrap_or(&path_str);
    PathBuf::from(stripped)
}

#[cfg(windows)]
pub fn path_starts_with_ignore_case<P: AsRef<Path>>(path: &Path, prefix: P) -> bool {
    let path_str = path.to_string_lossy().to_lowercase();
    let prefix_str = prefix.as_ref().to_string_lossy().to_lowercase();
    path_str.starts_with(&prefix_str)
}

#[cfg(not(windows))]
pub fn path_starts_with_ignore_case<P: AsRef<Path>>(path: &Path, prefix: P) -> bool {
    path.starts_with(prefix.as_ref())
}

#[cfg(windows)]
pub fn paths_equal_ignore_case<P1: AsRef<Path>, P2: AsRef<Path>>(path1: P1, path2: P2) -> bool {
    let str1 = path1.as_ref().to_string_lossy().to_lowercase();
    let str2 = path2.as_ref().to_string_lossy().to_lowercase();
    str1 == str2
}

#[cfg(not(windows))]
pub fn paths_equal_ignore_case<P1: AsRef<Path>, P2: AsRef<Path>>(path1: P1, path2: P2) -> bool {
    path1.as_ref() == path2.as_ref()
}

pub fn to_ftp_path(path: &Path, home_dir: &Path) -> Result<String, PathResolveError> {
    let normalized_path = normalize_windows_path(path);
    let normalized_home = normalize_windows_path(home_dir);
    
    let relative = match normalized_path.strip_prefix(&normalized_home) {
        Ok(r) => r,
        Err(_) => {
            log::warn!(
                "to_ftp_path: path {:?} is not under home_dir {:?} (normalized: path={:?}, home={:?})",
                path,
                home_dir,
                normalized_path,
                normalized_home
            );
            return Err(PathResolveError::PathNotUnderHome);
        }
    };
    let path_str = relative.to_string_lossy();
    let normalized = path_str.replace('\\', "/");
    if normalized.is_empty() || normalized == "." {
        Ok("/".to_string())
    } else {
        Ok(format!("/{}", normalized.trim_start_matches('/')))
    }
}

fn is_absolute_ftp_path(path: &str) -> bool {
    if path.is_empty() {
        return false;
    }
    
    let path_buf = Path::new(path);
    if path_buf.is_absolute() {
        return true;
    }
    
    let first_char = path.chars().next().unwrap();
    first_char == '/' || first_char == '\\'
}

fn is_valid_path_component(name: &std::ffi::OsStr) -> bool {
    let name_str = name.to_string_lossy();
    let name_str = name_str.trim();
    
    if name_str.is_empty() {
        return false;
    }
    
    if name_str == "." || name_str == ".." {
        return true;
    }
    
    if name_str.contains(':') {
        return false;
    }
    
    if cfg!(windows) {
        let invalid_chars = ['<', '>', '"', '|', '?', '*'];
        if name_str.chars().any(|c| invalid_chars.contains(&c)) {
            return false;
        }
        
        let reserved_names = [
            "CON", "PRN", "AUX", "NUL",
            "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9",
            "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
        ];
        let upper = name_str.to_uppercase();
        if reserved_names.contains(&upper.as_str()) {
            return false;
        }
    }
    
    true
}

fn resolve_path_internal(
    cwd: &str,
    home_canon: &Path,
    path: &str,
) -> Result<PathBuf, PathResolveError> {
    let clean_path = path.trim();

    if clean_path.is_empty() || clean_path == "." || clean_path == "./" {
        return Ok(home_canon.to_path_buf());
    }

    if clean_path.starts_with("\\\\?\\") {
        let path_buf = PathBuf::from(clean_path);
        if !path_buf.starts_with(home_canon) {
            log::warn!(
                "resolve_path_internal: Windows extended path outside home - input: {:?}, home: {:?}",
                clean_path,
                home_canon
            );
            return Err(PathResolveError::PathEscape);
        }
        return Ok(path_buf);
    }

    if is_absolute_ftp_path(clean_path) {
        let relative = clean_path
            .trim_start_matches('/')
            .trim_start_matches('\\');
        if relative.is_empty() {
            return Ok(home_canon.to_path_buf());
        }
        
        let relative_path = Path::new(relative);
        for component in relative_path.components() {
            if let Component::Prefix(_) | Component::RootDir = component {
                log::warn!(
                    "resolve_path_internal: Invalid prefix/root in absolute path - input: {:?}",
                    clean_path
                );
                return Err(PathResolveError::InvalidPath);
            }
            if let Component::Normal(name) = component
                && !is_valid_path_component(name)
            {
                log::warn!(
                    "resolve_path_internal: Invalid component in path - component: {:?}, input: {:?}",
                    name,
                    clean_path
                );
                return Err(PathResolveError::InvalidPath);
            }
        }
        
        let resolved_path = home_canon.join(relative);
        
        if !resolved_path.starts_with(home_canon) {
            log::warn!(
                "resolve_path_internal: Absolute path outside home directory - resolved: {:?}, home: {:?}, input: {:?}",
                resolved_path,
                home_canon,
                clean_path
            );
            return Err(PathResolveError::PathEscape);
        }
        
        Ok(resolved_path)
    } else {
        let clean_path_path = Path::new(clean_path);
        for component in clean_path_path.components() {
            if let Component::Prefix(_) | Component::RootDir = component {
                log::warn!(
                    "resolve_path_internal: Invalid prefix/root in relative path - input: {:?}",
                    clean_path
                );
                return Err(PathResolveError::InvalidPath);
            }
            if let Component::Normal(name) = component
                && !is_valid_path_component(name)
            {
                log::warn!(
                    "resolve_path_internal: Invalid component in relative path - component: {:?}, input: {:?}",
                    name,
                    clean_path
                );
                return Err(PathResolveError::InvalidPath);
            }
        }
        
        let base = if cwd.is_empty() {
            home_canon.to_path_buf()
        } else {
            let cwd_path = PathBuf::from(cwd);
            if !path_starts_with_ignore_case(&cwd_path, home_canon) {
                log::warn!(
                    "resolve_path_internal: CWD outside home - cwd: {:?}, home: {:?}",
                    cwd,
                    home_canon.display()
                );
                return Err(PathResolveError::PathEscape);
            }
            cwd_path
        };
        
        let resolved_path = base.join(clean_path);
        
        Ok(resolved_path)
    }
}

fn build_safe_path(
    home_canon: &Path,
    resolved: &Path,
    input_desc: &str,
) -> Result<PathBuf, PathResolveError> {
    let mut safe_path = home_canon.to_path_buf();
    let home_components: Vec<_> = home_canon.components().collect();
    let resolved_components: Vec<_> = resolved.components().collect();
    
    let mut relative_depth: usize = 0;
    
    for (i, component) in resolved_components.iter().enumerate() {
        match component {
            Component::Prefix(_) | Component::RootDir => {
                continue;
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if safe_path == *home_canon {
                    log::warn!(
                        "build_safe_path: Path escape via parent dir at root - input: {:?}, resolved: {:?}",
                        input_desc,
                        resolved
                    );
                    return Err(PathResolveError::PathEscape);
                }
                
                if !safe_path.pop() {
                    log::warn!(
                        "build_safe_path: Failed to pop path - input: {:?}, resolved: {:?}",
                        input_desc,
                        resolved
                    );
                    return Err(PathResolveError::PathEscape);
                }
                
                if !path_starts_with_ignore_case(&safe_path, home_canon) {
                    log::warn!(
                        "build_safe_path: Path escape after pop - input: {:?}, safe_path: {:?}, home: {:?}",
                        input_desc,
                        safe_path,
                        home_canon
                    );
                    return Err(PathResolveError::PathEscape);
                }
                
                relative_depth = relative_depth.saturating_sub(1);
            }
            Component::Normal(name) => {
                if i < home_components.len() {
                    continue;
                }
                
                if !is_valid_path_component(name) {
                    log::warn!(
                        "build_safe_path: Invalid component name - component: {:?}, input: {:?}",
                        name,
                        input_desc
                    );
                    return Err(PathResolveError::InvalidPath);
                }
                
                safe_path.push(name);
                relative_depth += 1;
                
                if relative_depth > MAX_PATH_DEPTH {
                    log::warn!(
                        "build_safe_path: Path depth exceeded - depth: {}, max: {}, input: {:?}",
                        relative_depth,
                        MAX_PATH_DEPTH,
                        input_desc
                    );
                    return Err(PathResolveError::PathTooDeep);
                }
            }
        }
    }
    
    if !path_starts_with_ignore_case(&safe_path, home_canon) {
        log::warn!(
            "build_safe_path: Final path outside home - safe_path: {:?}, home: {:?}, input: {:?}",
            safe_path,
            home_canon,
            input_desc
        );
        return Err(PathResolveError::PathEscape);
    }
    
    Ok(safe_path)
}

fn canonicalize_and_validate(
    path: &Path,
    home_canon: &Path,
    input_desc: &str,
    allow_symlink: bool,
) -> Result<PathBuf, PathResolveError> {
    match path.canonicalize() {
        Ok(canon) => {
            if !path_starts_with_ignore_case(&canon, home_canon) {
                log::warn!(
                    "canonicalize_and_validate: Path escape detected - canonicalized: {:?}, home: {:?}, input: {:?}",
                    canon,
                    home_canon,
                    input_desc
                );
                return Err(PathResolveError::PathEscape);
            }
            
            if !allow_symlink
                && let Ok(metadata) = path.symlink_metadata()
                && metadata.file_type().is_symlink()
            {
                log::warn!(
                    "canonicalize_and_validate: Symlink not allowed - path: {:?}, input: {:?}",
                    path,
                    input_desc
                );
                return Err(PathResolveError::SymlinkNotAllowed);
            }
            
            Ok(canon)
        }
        Err(e) => {
            log::debug!(
                "canonicalize_and_validate: Canonicalize failed - path: {:?}, error: {}, input: {:?}",
                path,
                e,
                input_desc
            );
            Err(PathResolveError::CanonicalizeFailed)
        }
    }
}

pub fn safe_resolve_path(
    cwd: &str,
    home_dir: &str,
    path: &str,
) -> Result<PathBuf, PathResolveError> {
    let input_desc = format!("cwd={}, home={}, path={}", cwd, home_dir, path);
    
    let home = PathBuf::from(home_dir);
    let home_canon = home.canonicalize().map_err(|e| {
        log::error!(
            "safe_resolve_path: Failed to canonicalize home directory - home: {:?}, error: {}",
            home_dir,
            e
        );
        PathResolveError::HomeDirectoryNotFound
    })?;

    let resolved = resolve_path_internal(cwd, &home_canon, path)?;
    
    match canonicalize_and_validate(&resolved, &home_canon, &input_desc, true) {
        Ok(canon) => Ok(canon),
        Err(PathResolveError::CanonicalizeFailed) => {
            let safe_path = build_safe_path(&home_canon, &resolved, &input_desc)?;
            Ok(safe_path)
        }
        Err(e) => Err(e),
    }
}

pub fn resolve_directory_path(
    cwd: &str,
    home_dir: &str,
    path: &str,
) -> Result<PathBuf, PathResolveError> {
    let input_desc = format!("cwd={}, home={}, path={}", cwd, home_dir, path);
    
    let home = PathBuf::from(home_dir);
    let home_canon = home.canonicalize().map_err(|e| {
        log::error!(
            "resolve_directory_path: Failed to canonicalize home directory - home: {:?}, error: {}",
            home_dir,
            e
        );
        PathResolveError::HomeDirectoryNotFound
    })?;

    let resolved = resolve_path_internal(cwd, &home_canon, path)?;
    
    match canonicalize_and_validate(&resolved, &home_canon, &input_desc, false) {
        Ok(canon) => {
            if !canon.is_dir() {
                log::warn!(
                    "resolve_directory_path: Path is not a directory - path: {:?}, input: {:?}",
                    canon,
                    input_desc
                );
                return Err(PathResolveError::NotADirectory);
            }
            Ok(canon)
        }
        Err(PathResolveError::CanonicalizeFailed) => {
            let safe_path = build_safe_path(&home_canon, &resolved, &input_desc)?;
            
            if let Ok(metadata) = safe_path.symlink_metadata()
                && metadata.file_type().is_symlink()
            {
                log::warn!(
                    "resolve_directory_path: Symlink detected in non-existent path - path: {:?}, input: {:?}",
                    safe_path,
                    input_desc
                );
                return Err(PathResolveError::SymlinkNotAllowed);
            }
            
            Ok(safe_path)
        }
        Err(e) => Err(e),
    }
}

pub fn safe_resolve_path_with_cwd(
    cwd: &str,
    home_dir: &str,
    path: &str,
) -> Result<PathBuf, PathResolveError> {
    safe_resolve_path(cwd, home_dir, path)
}

pub fn safe_resolve_path_no_symlink(
    cwd: &str,
    home_dir: &str,
    path: &str,
) -> Result<PathBuf, PathResolveError> {
    let input_desc = format!("cwd={}, home={}, path={}", cwd, home_dir, path);
    
    let home = PathBuf::from(home_dir);
    let home_canon = home.canonicalize().map_err(|e| {
        log::error!(
            "safe_resolve_path_no_symlink: Failed to canonicalize home directory - home: {:?}, error: {}",
            home_dir,
            e
        );
        PathResolveError::HomeDirectoryNotFound
    })?;

    let resolved = resolve_path_internal(cwd, &home_canon, path)?;
    
    match canonicalize_and_validate(&resolved, &home_canon, &input_desc, false) {
        Ok(canon) => Ok(canon),
        Err(PathResolveError::CanonicalizeFailed) => {
            let safe_path = build_safe_path(&home_canon, &resolved, &input_desc)?;
            
            if let Ok(metadata) = safe_path.symlink_metadata()
                && metadata.file_type().is_symlink()
            {
                log::warn!(
                    "safe_resolve_path_no_symlink: Symlink detected in non-existent path - path: {:?}, input: {:?}",
                    safe_path,
                    input_desc
                );
                return Err(PathResolveError::SymlinkNotAllowed);
            }
            
            Ok(safe_path)
        }
        Err(e) => Err(e),
    }
}

pub fn validate_existing_path(
    path: &Path,
    home_canon: &Path,
) -> Result<PathBuf, PathResolveError> {
    let canon = path.canonicalize().map_err(|e| {
        log::warn!(
            "validate_existing_path: Canonicalize failed - path: {:?}, error: {}",
            path,
            e
        );
        PathResolveError::CanonicalizeFailed
    })?;
    
    if !canon.starts_with(home_canon) {
        log::warn!(
            "validate_existing_path: Path escape detected - canonicalized: {:?}, home: {:?}",
            canon,
            home_canon
        );
        return Err(PathResolveError::PathEscape);
    }
    
    Ok(canon)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_absolute_ftp_path() {
        assert!(is_absolute_ftp_path("/home/user"));
        assert!(is_absolute_ftp_path("\\home\\user"));
        assert!(is_absolute_ftp_path("/"));
        assert!(!is_absolute_ftp_path("relative/path"));
        assert!(!is_absolute_ftp_path(""));
    }

    #[test]
    fn test_is_valid_path_component() {
        assert!(is_valid_path_component(std::ffi::OsStr::new("valid_name")));
        assert!(is_valid_path_component(std::ffi::OsStr::new("..")));
        assert!(is_valid_path_component(std::ffi::OsStr::new(".")));
        assert!(!is_valid_path_component(std::ffi::OsStr::new("invalid:name")));
        assert!(!is_valid_path_component(std::ffi::OsStr::new("")));
    }

    #[test]
    fn test_to_ftp_path_windows() {
        let home = Path::new("C:\\share_test");
        assert_eq!(to_ftp_path(Path::new("C:\\share_test\\file.txt"), home).unwrap(), "/file.txt");
        assert_eq!(to_ftp_path(Path::new("C:\\share_test"), home).unwrap(), "/");
        assert_eq!(to_ftp_path(Path::new("C:\\share_test\\subdir\\file.txt"), home).unwrap(), "/subdir/file.txt");
    }

    #[test]
    fn test_resolve_path_internal_absolute() {
        let home = PathBuf::from("C:\\share_test");
        if home.exists() {
            let home_canon = home.canonicalize().unwrap();
            
            let result = resolve_path_internal("", &home_canon, "/subdir/file.txt").unwrap();
            assert!(result.starts_with(&home_canon));
            assert!(result.to_string_lossy().contains("subdir"));
            
            let result2 = resolve_path_internal("", &home_canon, "/").unwrap();
            assert_eq!(result2, home_canon);
        }
    }

    #[test]
    fn test_resolve_path_internal_relative() {
        let home = PathBuf::from("C:\\share_test");
        if home.exists() {
            let home_canon = home.canonicalize().unwrap();
            
            let result = resolve_path_internal("", &home_canon, "file.txt").unwrap();
            assert!(result.starts_with(&home_canon));
            assert!(result.to_string_lossy().ends_with("file.txt"));
        }
    }
}
