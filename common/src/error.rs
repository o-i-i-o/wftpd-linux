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
