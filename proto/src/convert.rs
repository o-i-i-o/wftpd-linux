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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_entry_json_roundtrip_preserves_fields() {
        let original = LogEntryJson {
            timestamp: "2026-09-08T12:00:00+08:00".into(),
            level: "WARN".into(),
            source: "wftpd".into(),
            message: "连接被拒绝".into(),
            client_ip: Some("10.0.0.1".into()),
            username: None,
            action: Some("LOGIN".into()),
        };

        let proto = LogEntry::from(original.clone());
        assert_eq!(proto.client_ip, "10.0.0.1");
        assert_eq!(proto.username, "", "None 序列化为空字符串");
        assert_eq!(proto.action, "LOGIN");

        let back = LogEntryJson::from(proto);
        assert_eq!(back, original);
    }

    #[test]
    fn log_entry_none_maps_to_empty_and_back() {
        let proto = LogEntry {
            timestamp: "t".into(),
            level: "INFO".into(),
            source: "s".into(),
            message: "m".into(),
            client_ip: String::new(),
            username: String::new(),
            action: String::new(),
        };
        let json = LogEntryJson::from(proto.clone());
        assert!(json.client_ip.is_none());
        assert!(json.username.is_none());
        assert!(json.action.is_none());
        // 空字符串与 None 往返后仍为 None
        assert_eq!(LogEntry::from(json), proto);
    }

    #[test]
    fn file_op_log_entry_roundtrip() {
        let original = FileLogEntryJson {
            timestamp: "2026-09-08T12:00:00+08:00".into(),
            username: "alice".into(),
            client_ip: "10.0.0.1".into(),
            operation: "UPLOAD".into(),
            file_path: "/a/b.txt".into(),
            file_size: 4096,
            protocol: "SFTP".into(),
            success: true,
            message: "文件上传成功".into(),
        };

        let proto = FileOpLogEntry::from(original.clone());
        assert_eq!(proto.file_size, 4096);
        assert!(proto.success);

        let back = FileLogEntryJson::from(proto);
        assert_eq!(back, original);
    }

    #[test]
    fn file_op_log_entry_failed_flag_roundtrip() {
        let json = FileLogEntryJson {
            timestamp: "t".into(),
            username: "u".into(),
            client_ip: "ip".into(),
            operation: "DELETE".into(),
            file_path: "/x".into(),
            file_size: 0,
            protocol: "FTP".into(),
            success: false,
            message: "权限不足".into(),
        };
        let back = FileLogEntryJson::from(FileOpLogEntry::from(json));
        assert!(!back.success);
    }

    #[test]
    fn log_file_entry_roundtrip() {
        let original = LogFileEntry {
            name: "wftpg.2026-09-08".into(),
            path: "/var/log/wftpd/wftpg.2026-09-08.log".into(),
        };
        let proto = LogFileInfo::from(original.clone());
        assert_eq!(proto.name, "wftpg.2026-09-08");
        assert_eq!(LogFileEntry::from(proto), original);
    }
}
