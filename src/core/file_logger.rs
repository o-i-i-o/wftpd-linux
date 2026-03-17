use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

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
    log_dir: PathBuf,
    buffer: Arc<Mutex<VecDeque<FileLogEntry>>>,
    max_buffer_size: usize,
    current_file: Option<File>,
    current_size: u64,
    max_file_size: u64,
}

impl FileLogger {
    pub fn new(log_dir: &str, max_file_size: u64) -> Self {
        let path = PathBuf::from(log_dir);
        
        if let Err(e) = fs::create_dir_all(&path) {
            eprintln!("Warning: Failed to create file log directory {}: {}", path.display(), e);
        }
        
        let (log_path, size) = Self::get_available_log_path(&path);
        let file = match OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            Ok(f) => Some(f),
            Err(e) => {
                eprintln!("Warning: Failed to open file log: {}", e);
                None
            }
        };

        FileLogger {
            log_dir: path,
            buffer: Arc::new(Mutex::new(VecDeque::with_capacity(2000))),
            max_buffer_size: 2000,
            current_file: file,
            current_size: size,
            max_file_size,
        }
    }

    fn get_available_log_path(log_dir: &Path) -> (PathBuf, u64) {
        let date_str = Local::now().format("%Y-%m-%d");
        let mut seq = 1;
        
        loop {
            let filename = format!("file-ops-{}-{:04}.log", date_str, seq);
            let log_path = log_dir.join(&filename);
            
            if !log_path.exists() {
                return (log_path, 0);
            }
            
            if let Ok(metadata) = fs::metadata(&log_path) {
                let size = metadata.len();
                if size < 2 * 1024 * 1024 {
                    return (log_path, size);
                }
            }
            
            seq += 1;
        }
    }

    fn get_new_log_path(&self) -> PathBuf {
        let date_str = Local::now().format("%Y-%m-%d");
        let mut seq = 1;
        
        loop {
            let filename = format!("file-ops-{}-{:04}.log", date_str, seq);
            let log_path = self.log_dir.join(&filename);
            
            if !log_path.exists() {
                return log_path;
            }
            
            seq += 1;
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
            buffer.push_back(entry.clone());
        }

        if let Err(e) = self.write_to_file(&entry) {
            eprintln!("Failed to write file log: {}", e);
        }
    }

    fn write_to_file(&mut self, entry: &FileLogEntry) -> std::io::Result<()> {
        if self.current_file.is_none() || self.current_size >= self.max_file_size {
            let new_path = self.get_new_log_path();
            self.current_file = Some(
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&new_path)?,
            );
            self.current_size = 0;
        }

        let json = serde_json::to_string(entry)
            .unwrap_or_else(|_| format!("{{\"message\": \"{}\"}}", entry.message));

        if let Some(ref mut file) = self.current_file {
            let line = format!("{}\n", json);
            let bytes = line.as_bytes();
            file.write_all(bytes)?;
            self.current_size += bytes.len() as u64;
        }

        Ok(())
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
