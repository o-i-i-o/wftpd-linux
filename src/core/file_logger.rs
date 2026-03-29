//! 文件操作日志模块
//! 
//! 基于 tracing 实现文件操作审计日志
//! - 日志通过 tracing 输出到 file_ops target
//! - 内存缓冲区用于 UI 实时显示

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tracing::info;

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
}

impl FileLogger {
    pub fn new(_log_dir: &str, _max_file_size: u64) -> Self {
        FileLogger {
            buffer: Arc::new(Mutex::new(VecDeque::with_capacity(2000))),
            max_buffer_size: 2000,
        }
    }

    pub fn log(&mut self, info: FileLogInfo<'_>) {
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

    pub fn get_recent_logs(&self, count: usize) -> Vec<FileLogEntry> {
        let buffer = self.buffer.lock().unwrap();
        buffer.iter().rev().take(count).cloned().collect()
    }

    pub fn get_buffer(&self) -> Arc<Mutex<VecDeque<FileLogEntry>>> {
        Arc::clone(&self.buffer)
    }

    pub fn log_upload(&mut self, username: &str, client_ip: &str, file_path: &str, file_size: u64, protocol: &str) {
        self.log(FileLogInfo {
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

    pub fn log_update(&mut self, username: &str, client_ip: &str, file_path: &str, file_size: u64, protocol: &str) {
        self.log(FileLogInfo {
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

    pub fn log_download(&mut self, username: &str, client_ip: &str, file_path: &str, file_size: u64, protocol: &str) {
        self.log(FileLogInfo {
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
        self.log(FileLogInfo {
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

    pub fn log_rename(&mut self, username: &str, client_ip: &str, old_path: &str, new_path: &str, protocol: &str) {
        self.log(FileLogInfo {
            username,
            client_ip,
            operation: "RENAME",
            file_path: &format!("{} -> {}", old_path, new_path),
            file_size: 0,
            protocol,
            success: true,
            message: "文件重命名成功",
        });
    }

    pub fn log_mkdir(&mut self, username: &str, client_ip: &str, dir_path: &str, protocol: &str) {
        self.log(FileLogInfo {
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
        self.log(FileLogInfo {
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

    pub fn log_failed(&mut self, username: &str, client_ip: &str, operation: &str, file_path: &str, protocol: &str, error: &str) {
        self.log(FileLogInfo {
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
