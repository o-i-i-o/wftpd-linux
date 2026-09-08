//! 文件操作日志模块
//!
//! 基于 tracing 实现文件操作审计日志
//! - 日志通过 tracing 输出到 `file_ops` target
//! - 内存缓冲区用于 UI 实时显示

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;
use tracing::info;

/// 日志条目 JSON 结构（用于网络传输）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogEntryJson {
    pub timestamp: String,
    pub level: String,
    pub source: String,
    pub message: String,
    pub client_ip: Option<String>,
    pub username: Option<String>,
    pub action: Option<String>,
}

/// 日志文件条目（供前端列出可选日志文件）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogFileEntry {
    pub name: String,
    pub path: String,
}

/// 文件操作日志条目（用于网络传输）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileLogEntryJson {
    pub timestamp: String,
    pub username: String,
    pub client_ip: String,
    pub operation: String,
    pub file_path: String,
    pub file_size: u64,
    pub protocol: String,
    pub success: bool,
    pub message: String,
}

impl From<&FileLogEntry> for FileLogEntryJson {
    fn from(e: &FileLogEntry) -> Self {
        FileLogEntryJson {
            timestamp: e.timestamp.to_rfc3339(),
            username: e.username.clone(),
            client_ip: e.client_ip.clone(),
            operation: e.operation.clone(),
            file_path: e.file_path.clone(),
            file_size: e.file_size,
            protocol: e.protocol.clone(),
            success: e.success,
            message: e.message.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileLogEntry {
    pub timestamp: DateTime<Local>,
    pub username: String,
    pub client_ip: String,
    pub operation: String,
    pub file_path: String,
    pub file_size: u64,
    pub protocol: String,
    pub success: bool,
    pub message: String,
}

pub struct FileLogInfo<'a> {
    pub username: &'a str,
    pub client_ip: &'a str,
    pub operation: &'a str,
    pub file_path: &'a str,
    pub file_size: u64,
    pub protocol: &'a str,
    pub success: bool,
    pub message: &'a str,
}

pub struct FileLogger {
    buffer: Arc<Mutex<VecDeque<FileLogEntry>>>,
    max_buffer_size: usize,
    log_sender: Option<broadcast::Sender<LogEntryJson>>,
}

impl Clone for FileLogger {
    fn clone(&self) -> Self {
        Self {
            buffer: Arc::clone(&self.buffer),
            max_buffer_size: self.max_buffer_size,
            log_sender: self.log_sender.clone(),
        }
    }
}

impl FileLogger {
    #[must_use]
    pub fn new(_log_dir: &str, _max_file_size: u64) -> Self {
        FileLogger {
            buffer: Arc::new(Mutex::new(VecDeque::with_capacity(2000))),
            max_buffer_size: 2000,
            log_sender: None,
        }
    }

    #[must_use]
    pub fn with_log_sender(log_sender: broadcast::Sender<LogEntryJson>) -> Self {
        FileLogger {
            buffer: Arc::new(Mutex::new(VecDeque::with_capacity(2000))),
            max_buffer_size: 2000,
            log_sender: Some(log_sender),
        }
    }

    /// 记录一条文件操作：推送广播、写入内存缓冲并输出到 `file_ops` target
    ///
    /// # Panics
    /// 内存缓冲互斥锁中毒（持有线程 panic）时 panic
    pub fn log(&mut self, info: &FileLogInfo<'_>) {
        let entry = FileLogEntry {
            timestamp: Local::now(),
            username: info.username.to_string(),
            client_ip: info.client_ip.to_string(),
            operation: info.operation.to_string(),
            file_path: info.file_path.to_string(),
            file_size: info.file_size,
            protocol: info.protocol.to_string(),
            success: info.success,
            message: info.message.to_string(),
        };

        // 推送到前端（如果启用了 GUI 日志）
        if let Some(ref sender) = self.log_sender {
            let log_entry_json = LogEntryJson {
                timestamp: entry.timestamp.to_rfc3339(),
                level: "INFO".to_string(),
                source: "文件操作".to_string(),
                message: format!("{} {} {}", entry.username, entry.operation, entry.file_path),
                client_ip: Some(entry.client_ip.clone()),
                username: Some(entry.username.clone()),
                action: Some(entry.operation.clone()),
            };
            let _ = sender.send(log_entry_json);
        }

        {
            let mut buffer = self.buffer.lock().unwrap();
            if buffer.len() >= self.max_buffer_size {
                buffer.pop_front();
            }
            buffer.push_back(entry);
        }

        info!(
            target: "file_ops",
            username = info.username,
            client_ip = info.client_ip,
            operation = info.operation,
            file_path = info.file_path,
            file_size = info.file_size,
            protocol = info.protocol,
            success = info.success,
            "{}",
            info.message
        );
    }

    #[must_use]
    /// 最近的 `count` 条文件操作记录（新→旧）
    ///
    /// # Panics
    /// 内存缓冲互斥锁中毒（持有线程 panic）时 panic
    pub fn get_recent_logs(&self, count: usize) -> Vec<FileLogEntry> {
        let buffer = self.buffer.lock().unwrap();
        buffer.iter().rev().take(count).cloned().collect()
    }

    #[must_use]
    pub fn get_buffer(&self) -> Arc<Mutex<VecDeque<FileLogEntry>>> {
        Arc::clone(&self.buffer)
    }

    pub fn log_upload(
        &mut self,
        username: &str,
        client_ip: &str,
        file_path: &str,
        file_size: u64,
        protocol: &str,
    ) {
        self.log(&FileLogInfo {
            username,
            client_ip,
            operation: "UPLOAD",
            file_path,
            file_size,
            protocol,
            success: true,
            message: "文件上传成功",
        });
    }

    pub fn log_update(
        &mut self,
        username: &str,
        client_ip: &str,
        file_path: &str,
        file_size: u64,
        protocol: &str,
    ) {
        self.log(&FileLogInfo {
            username,
            client_ip,
            operation: "UPDATE",
            file_path,
            file_size,
            protocol,
            success: true,
            message: "文件更新成功",
        });
    }

    pub fn log_download(
        &mut self,
        username: &str,
        client_ip: &str,
        file_path: &str,
        file_size: u64,
        protocol: &str,
    ) {
        self.log(&FileLogInfo {
            username,
            client_ip,
            operation: "DOWNLOAD",
            file_path,
            file_size,
            protocol,
            success: true,
            message: "文件下载成功",
        });
    }

    pub fn log_delete(&mut self, username: &str, client_ip: &str, file_path: &str, protocol: &str) {
        self.log(&FileLogInfo {
            username,
            client_ip,
            operation: "DELETE",
            file_path,
            file_size: 0,
            protocol,
            success: true,
            message: "文件删除成功",
        });
    }

    pub fn log_rename(
        &mut self,
        username: &str,
        client_ip: &str,
        old_path: &str,
        new_path: &str,
        protocol: &str,
    ) {
        self.log(&FileLogInfo {
            username,
            client_ip,
            operation: "RENAME",
            file_path: &format!("{old_path} -> {new_path}"),
            file_size: 0,
            protocol,
            success: true,
            message: "文件重命名成功",
        });
    }

    pub fn log_mkdir(&mut self, username: &str, client_ip: &str, dir_path: &str, protocol: &str) {
        self.log(&FileLogInfo {
            username,
            client_ip,
            operation: "MKDIR",
            file_path: dir_path,
            file_size: 0,
            protocol,
            success: true,
            message: "目录创建成功",
        });
    }

    pub fn log_rmdir(&mut self, username: &str, client_ip: &str, dir_path: &str, protocol: &str) {
        self.log(&FileLogInfo {
            username,
            client_ip,
            operation: "RMDIR",
            file_path: dir_path,
            file_size: 0,
            protocol,
            success: true,
            message: "目录删除成功",
        });
    }

    pub fn log_failed(
        &mut self,
        username: &str,
        client_ip: &str,
        operation: &str,
        file_path: &str,
        protocol: &str,
        error: &str,
    ) {
        self.log(&FileLogInfo {
            username,
            client_ip,
            operation,
            file_path,
            file_size: 0,
            protocol,
            success: false,
            message: error,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn logger() -> FileLogger {
        FileLogger::new("/tmp", 0)
    }

    #[test]
    fn convenience_methods_record_expected_operations() {
        let mut fl = logger();
        fl.log_upload("u", "1.1.1.1", "/a", 10, "FTP");
        fl.log_update("u", "1.1.1.1", "/a", 12, "FTP");
        fl.log_download("u", "1.1.1.1", "/a", 12, "FTP");
        fl.log_delete("u", "1.1.1.1", "/a", "FTP");
        fl.log_rename("u", "1.1.1.1", "/a", "/b", "FTP");
        fl.log_mkdir("u", "1.1.1.1", "/d", "FTP");
        fl.log_rmdir("u", "1.1.1.1", "/d", "FTP");
        fl.log_failed("u", "1.1.1.1", "UPLOAD", "/a", "FTP", "磁盘已满");

        let logs = fl.get_recent_logs(100);
        assert_eq!(logs.len(), 8);
        // get_recent_logs 新→旧排列
        let ops: Vec<&str> = logs.iter().map(|e| e.operation.as_str()).collect();
        assert_eq!(
            ops,
            vec![
                "UPLOAD", "RMDIR", "MKDIR", "RENAME", "DELETE", "DOWNLOAD", "UPDATE", "UPLOAD"
            ]
        );
        assert!(logs[0].message.contains("磁盘已满"));
        assert!(!logs[0].success);
        assert!(logs[1].success);
    }

    #[test]
    fn entry_fields_preserved() {
        let mut fl = logger();
        fl.log_upload("alice", "10.0.0.9", "/up/x.bin", 4096, "SFTP");

        let entry = &fl.get_recent_logs(1)[0];
        assert_eq!(entry.username, "alice");
        assert_eq!(entry.client_ip, "10.0.0.9");
        assert_eq!(entry.file_path, "/up/x.bin");
        assert_eq!(entry.file_size, 4096);
        assert_eq!(entry.protocol, "SFTP");
    }

    #[test]
    fn rename_records_both_paths() {
        let mut fl = logger();
        fl.log_rename("u", "ip", "/old", "/new", "FTP");
        assert_eq!(fl.get_recent_logs(1)[0].file_path, "/old -> /new");
    }

    #[test]
    fn get_recent_logs_limits_count() {
        let mut fl = logger();
        for i in 0..10 {
            fl.log_mkdir("u", "ip", &format!("/d{i}"), "FTP");
        }
        let logs = fl.get_recent_logs(3);
        assert_eq!(logs.len(), 3);
        assert_eq!(logs[0].file_path, "/d9");
        assert_eq!(logs[2].file_path, "/d7");
    }

    #[test]
    fn clone_shares_underlying_buffer() {
        let mut fl = logger();
        let snapshot = fl.clone();
        fl.log_upload("u", "ip", "/a", 1, "FTP");
        assert_eq!(snapshot.get_recent_logs(10).len(), 1);
    }

    #[test]
    fn json_conversion_serializes_fields() {
        let mut fl = logger();
        fl.log_upload("alice", "10.0.0.9", "/a", 7, "FTP");
        let entry = &fl.get_recent_logs(1)[0];

        let json = FileLogEntryJson::from(entry);
        assert_eq!(json.username, "alice");
        assert_eq!(json.file_size, 7);
        assert!(json.success);
        assert!(json.timestamp.contains('T'), "时间戳应为 RFC3339 格式");

        // JSON 往返无损
        let restored: FileLogEntryJson =
            serde_json::from_str(&serde_json::to_string(&json).unwrap()).unwrap();
        assert_eq!(restored.file_path, json.file_path);
        assert_eq!(restored.operation, json.operation);
    }
}
