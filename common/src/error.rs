use std::fmt;

#[derive(Debug, Clone)]
pub enum WftpgError {
    ConfigError(String),
    NetworkError(String),
    AuthError(String),
    FileError(String),
    PermissionError(String),
    UserError(String),
    ProtocolError(String),
    InternalError(String),
    PathResolveError(String),
}

impl fmt::Display for WftpgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WftpgError::ConfigError(msg) => write!(f, "配置错误: {msg}"),
            WftpgError::NetworkError(msg) => write!(f, "网络错误: {msg}"),
            WftpgError::AuthError(msg) => write!(f, "认证错误: {msg}"),
            WftpgError::FileError(msg) => write!(f, "文件错误: {msg}"),
            WftpgError::PermissionError(msg) => write!(f, "权限错误: {msg}"),
            WftpgError::UserError(msg) => write!(f, "用户错误: {msg}"),
            WftpgError::ProtocolError(msg) => write!(f, "协议错误: {msg}"),
            WftpgError::InternalError(msg) => write!(f, "内部错误: {msg}"),
            WftpgError::PathResolveError(msg) => write!(f, "路径解析错误: {msg}"),
        }
    }
}

impl std::error::Error for WftpgError {}

impl From<std::io::Error> for WftpgError {
    fn from(e: std::io::Error) -> Self {
        WftpgError::FileError(e.to_string())
    }
}

impl From<toml::de::Error> for WftpgError {
    fn from(e: toml::de::Error) -> Self {
        WftpgError::ConfigError(e.to_string())
    }
}

impl From<serde_json::Error> for WftpgError {
    fn from(e: serde_json::Error) -> Self {
        WftpgError::ConfigError(e.to_string())
    }
}

pub type WftpgResult<T> = Result<T, WftpgError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_includes_kind_prefix() {
        assert_eq!(
            WftpgError::ConfigError("bad".into()).to_string(),
            "配置错误: bad"
        );
        assert_eq!(
            WftpgError::PathResolveError("escape".into()).to_string(),
            "路径解析错误: escape"
        );
        assert_eq!(
            WftpgError::AuthError("denied".into()).to_string(),
            "认证错误: denied"
        );
    }

    #[test]
    fn all_variants_display_without_panic() {
        let samples = [
            WftpgError::ConfigError("x".into()),
            WftpgError::NetworkError("x".into()),
            WftpgError::AuthError("x".into()),
            WftpgError::FileError("x".into()),
            WftpgError::PermissionError("x".into()),
            WftpgError::UserError("x".into()),
            WftpgError::ProtocolError("x".into()),
            WftpgError::InternalError("x".into()),
            WftpgError::PathResolveError("x".into()),
        ];
        for e in &samples {
            assert!(!e.to_string().is_empty());
            let _ = format!("{e:?}");
        }
    }

    #[test]
    fn from_io_error_maps_to_file_error() {
        let err = WftpgError::from(std::io::Error::new(std::io::ErrorKind::NotFound, "gone"));
        assert!(matches!(err, WftpgError::FileError(_)));
    }

    #[test]
    fn from_serde_errors_map_to_config_error() {
        let json_err = serde_json::from_str::<String>("{").unwrap_err();
        assert!(matches!(
            WftpgError::from(json_err),
            WftpgError::ConfigError(_)
        ));

        let toml_err = toml::from_str::<String>("=").unwrap_err();
        assert!(matches!(
            WftpgError::from(toml_err),
            WftpgError::ConfigError(_)
        ));
    }
}
