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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_level_display() {
        assert_eq!(LogLevel::Debug.to_string(), "DEBUG");
        assert_eq!(LogLevel::Info.to_string(), "INFO");
        assert_eq!(LogLevel::Warning.to_string(), "WARN");
        assert_eq!(LogLevel::Error.to_string(), "ERROR");
    }

    #[test]
    fn basic_levels_recorded_with_source() {
        let logger = Logger::new("/tmp", 0, 1);
        logger.debug("src", "d");
        logger.info("src", "i");
        logger.warning("src", "w");
        logger.error("src", "e");

        let logs = logger.get_recent_logs(10);
        assert_eq!(logs.len(), 4);
        assert_eq!(logs[0].level, LogLevel::Error, "最近一条应为 error");
        assert_eq!(logs[3].level, LogLevel::Debug);
        assert!(logs.iter().all(|e| e.source == "src"));
        assert!(logs.iter().all(|e| e.client_ip.is_none()));
    }

    #[test]
    fn client_action_records_context() {
        let logger = Logger::new("/tmp", 0, 1);
        logger.client_action("ftp", "登录成功", "1.2.3.4", Some("alice"), "LOGIN");

        let logs = logger.get_recent_logs(1);
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].client_ip.as_deref(), Some("1.2.3.4"));
        assert_eq!(logs[0].username.as_deref(), Some("alice"));
        assert_eq!(logs[0].action.as_deref(), Some("LOGIN"));
        assert_eq!(logs[0].message, "登录成功");
    }

    #[test]
    fn get_recent_logs_returns_newest_first_and_limits_count() {
        let logger = Logger::new("/tmp", 0, 1);
        logger.info("a", "1");
        logger.info("a", "2");
        logger.info("a", "3");

        let two = logger.get_recent_logs(2);
        assert_eq!(two.len(), 2);
        assert_eq!(two[0].message, "3");
        assert_eq!(two[1].message, "2");
    }

    #[test]
    fn buffer_is_bounded() {
        let logger = Logger::new("/tmp", 0, 1);
        for i in 0..1100 {
            logger.info("a", &format!("{i}"));
        }
        let logs = logger.get_recent_logs(2000);
        assert!(logs.len() <= 1000, "缓冲区应保持有界，实际 {}", logs.len());
        assert_eq!(logs[0].message, "1099");
    }
}
