//! proto 消息与 wftpd-common 领域类型之间的转换。
//!
//! proto3 的 string 不能表达 null，约定：空字符串等价于"无值"，
//! 转回 [`wftpd_common`] 类型时映射为 `None`。

use wftpd_common::{FileLogEntryJson, LogEntryJson, LogFileEntry};

use crate::wftpd::v1::{FileOpLogEntry, LogEntry, LogFileInfo};

fn opt_to_str(v: Option<&str>) -> String {
    v.unwrap_or_default().to_string()
}

fn str_to_opt(v: String) -> Option<String> {
    if v.is_empty() { None } else { Some(v) }
}

impl From<LogEntryJson> for LogEntry {
    fn from(e: LogEntryJson) -> Self {
        LogEntry {
            timestamp: e.timestamp,
            level: e.level,
            source: e.source,
            message: e.message,
            client_ip: opt_to_str(e.client_ip.as_deref()),
            username: opt_to_str(e.username.as_deref()),
            action: opt_to_str(e.action.as_deref()),
        }
    }
}

impl From<LogEntry> for LogEntryJson {
    fn from(e: LogEntry) -> Self {
        LogEntryJson {
            timestamp: e.timestamp,
            level: e.level,
            source: e.source,
            message: e.message,
            client_ip: str_to_opt(e.client_ip),
            username: str_to_opt(e.username),
            action: str_to_opt(e.action),
        }
    }
}

impl From<FileLogEntryJson> for FileOpLogEntry {
    fn from(e: FileLogEntryJson) -> Self {
        FileOpLogEntry {
            timestamp: e.timestamp,
            username: e.username,
            client_ip: e.client_ip,
            operation: e.operation,
            file_path: e.file_path,
            file_size: e.file_size,
            protocol: e.protocol,
            success: e.success,
            message: e.message,
        }
    }
}

impl From<FileOpLogEntry> for FileLogEntryJson {
    fn from(e: FileOpLogEntry) -> Self {
        FileLogEntryJson {
            timestamp: e.timestamp,
            username: e.username,
            client_ip: e.client_ip,
            operation: e.operation,
            file_path: e.file_path,
            file_size: e.file_size,
            protocol: e.protocol,
            success: e.success,
            message: e.message,
        }
    }
}

impl From<LogFileEntry> for LogFileInfo {
    fn from(e: LogFileEntry) -> Self {
        LogFileInfo {
            name: e.name,
            path: e.path,
        }
    }
}

impl From<LogFileInfo> for LogFileEntry {
    fn from(e: LogFileInfo) -> Self {
        LogFileEntry {
            name: e.name,
            path: e.path,
        }
    }
}
