//! Logger 兼容层 - 内部使用 tracing 实现
//!
//! 这个模块提供了一个与旧 Logger 兼容的接口，但内部使用 tracing 实现
//! 所有日志输出都通过 tracing 系统进行

use std::sync::Mutex;
use tracing::{debug, error, info, warn};

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub timestamp: chrono::DateTime<chrono::Local>,
    pub level: LogLevel,
    pub source: String,
    pub message: String,
    pub client_ip: Option<String>,
    pub username: Option<String>,
    pub action: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Debug,
    Info,
    Warning,
    Error,
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogLevel::Debug => write!(f, "DEBUG"),
            LogLevel::Info => write!(f, "INFO"),
            LogLevel::Warning => write!(f, "WARN"),
            LogLevel::Error => write!(f, "ERROR"),
        }
    }
}

pub struct Logger {
    buffer: Mutex<Vec<LogEntry>>,
    max_buffer_size: usize,
}

impl Logger {
    #[must_use]
    pub fn new(_log_dir: &str, _max_file_size: u64, _max_files: usize) -> Self {
        Logger {
            buffer: Mutex::new(Vec::with_capacity(1000)),
            max_buffer_size: 1000,
        }
    }

    pub fn debug(&self, source: &str, message: &str) {
        self.log(LogLevel::Debug, source, message, None, None, None);
        debug!(source = source, "{}", message);
    }

    pub fn info(&self, source: &str, message: &str) {
        self.log(LogLevel::Info, source, message, None, None, None);
        info!(source = source, "{}", message);
    }

    pub fn warning(&self, source: &str, message: &str) {
        self.log(LogLevel::Warning, source, message, None, None, None);
        warn!(source = source, "{}", message);
    }

    pub fn error(&self, source: &str, message: &str) {
        self.log(LogLevel::Error, source, message, None, None, None);
        error!(source = source, "{}", message);
    }

    pub fn client_action(
        &self,
        source: &str,
        message: &str,
        client_ip: &str,
        username: Option<&str>,
        action: &str,
    ) {
        self.log(
            LogLevel::Info,
            source,
            message,
            Some(client_ip),
            username,
            Some(action),
        );
        info!(
            source = source,
            client_ip = client_ip,
            username = username.unwrap_or("anonymous"),
            action = action,
            "{}",
            message
        );
    }

    fn log(
        &self,
        level: LogLevel,
        source: &str,
        message: &str,
        client_ip: Option<&str>,
        username: Option<&str>,
        action: Option<&str>,
    ) {
        let entry = LogEntry {
            timestamp: chrono::Local::now(),
            level,
            source: source.to_string(),
            message: message.to_string(),
            client_ip: client_ip.map(std::string::ToString::to_string),
            username: username.map(std::string::ToString::to_string),
            action: action.map(std::string::ToString::to_string),
        };

        if let Ok(mut buffer) = self.buffer.lock() {
            if buffer.len() >= self.max_buffer_size {
                buffer.remove(0);
            }
            buffer.push(entry);
        }
    }

    pub fn get_recent_logs(&self, count: usize) -> Vec<LogEntry> {
        if let Ok(buffer) = self.buffer.lock() {
            buffer.iter().rev().take(count).cloned().collect()
        } else {
            Vec::new()
        }
    }

    pub fn set_level(&self, _level: &str) {
        // Level is managed by tracing subscriber
    }
}
