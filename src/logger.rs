use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::collections::VecDeque;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp: DateTime<Utc>,
    pub level: LogLevel,
    pub source: String,
    pub message: String,
    pub client_ip: Option<String>,
    pub username: Option<String>,
    pub action: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
    log_dir: PathBuf,
    max_size: u64,
    max_files: usize,
    current_file: Option<File>,
    current_size: u64,
    buffer: Arc<Mutex<VecDeque<LogEntry>>>,
    max_buffer_size: usize,
}

impl Logger {
    pub fn new(log_dir: &str, max_size: u64, max_files: usize) -> Self {
        let path = PathBuf::from(log_dir);
        
        Logger {
            log_dir: path,
            max_size,
            max_files,
            current_file: None,
            current_size: 0,
            buffer: Arc::new(Mutex::new(VecDeque::with_capacity(1000))),
            max_buffer_size: 1000,
        }
    }
    
    pub fn init(&mut self) -> std::io::Result<()> {
        fs::create_dir_all(&self.log_dir)?;
        self.rotate_if_needed()?;
        Ok(())
    }
    
    fn get_log_file_path(&self) -> PathBuf {
        self.log_dir.join(format!("wftpg-{}.log", 
            Utc::now().format("%Y-%m-%d")))
    }
    
    fn rotate_if_needed(&mut self) -> std::io::Result<()> {
        let log_path = self.get_log_file_path();
        
        if log_path.exists() {
            let metadata = fs::metadata(&log_path)?;
            self.current_size = metadata.len();
            
            if self.current_size >= self.max_size {
                self.rotate_logs()?;
                self.current_size = 0;
            }
        }
        
        self.current_file = Some(OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)?);
        
        Ok(())
    }
    
    fn rotate_logs(&mut self) -> std::io::Result<()> {
        let log_path = self.get_log_file_path();
        let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
        let rotated_path = self.log_dir.join(format!("wftpg-{}.log", timestamp));
        
        if log_path.exists() {
            fs::rename(&log_path, &rotated_path)?;
        }
        
        self.cleanup_old_logs()?;
        
        Ok(())
    }
    
    fn cleanup_old_logs(&self) -> std::io::Result<()> {
        let mut log_files: Vec<_> = fs::read_dir(&self.log_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("wftpg-"))
            .collect();
        
        log_files.sort_by_key(|e| e.file_name());
        
        while log_files.len() > self.max_files {
            if let Some(old_file) = log_files.first() {
                fs::remove_file(old_file.path())?;
                log_files.remove(0);
            }
        }
        
        Ok(())
    }
    
    pub fn log(&mut self, level: LogLevel, source: &str, message: &str, 
               client_ip: Option<&str>, username: Option<&str>, action: Option<&str>) {
        let entry = LogEntry {
            timestamp: Utc::now(),
            level: level.clone(),
            source: source.to_string(),
            message: message.to_string(),
            client_ip: client_ip.map(|s| s.to_string()),
            username: username.map(|s| s.to_string()),
            action: action.map(|s| s.to_string()),
        };
        
        {
            let mut buffer = self.buffer.lock().unwrap();
            if buffer.len() >= self.max_buffer_size {
                buffer.pop_front();
            }
            buffer.push_back(entry.clone());
        }
        
        if let Err(e) = self.write_to_file(&entry) {
            eprintln!("Failed to write log: {}", e);
        }
        
        println!("[{}] [{}] {} - {}", 
            entry.timestamp.format("%Y-%m-%d %H:%M:%S"),
            entry.level,
            entry.source,
            entry.message);
    }
    
    fn write_to_file(&mut self, entry: &LogEntry) -> std::io::Result<()> {
        if self.current_file.is_none() || self.current_size >= self.max_size {
            self.rotate_if_needed()?;
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
    
    pub fn get_recent_logs(&self, count: usize) -> Vec<LogEntry> {
        let buffer = self.buffer.lock().unwrap();
        buffer.iter().rev().take(count).cloned().collect()
    }
    
    pub fn get_all_buffered_logs(&self) -> Vec<LogEntry> {
        let buffer = self.buffer.lock().unwrap();
        buffer.iter().cloned().collect()
    }
    
    pub fn get_log_buffer_arc(&self) -> Arc<Mutex<VecDeque<LogEntry>>> {
        Arc::clone(&self.buffer)
    }
    
    pub fn debug(&mut self, source: &str, message: &str) {
        self.log(LogLevel::Debug, source, message, None, None, None);
    }
    
    pub fn info(&mut self, source: &str, message: &str) {
        self.log(LogLevel::Info, source, message, None, None, None);
    }
    
    pub fn warning(&mut self, source: &str, message: &str) {
        self.log(LogLevel::Warning, source, message, None, None, None);
    }
    
    pub fn error(&mut self, source: &str, message: &str) {
        self.log(LogLevel::Error, source, message, None, None, None);
    }
    
    pub fn client_action(&mut self, source: &str, message: &str, 
                         client_ip: &str, username: Option<&str>, action: &str) {
        self.log(LogLevel::Info, source, message, Some(client_ip), username, Some(action));
    }
}

pub fn read_log_file(log_dir: &str, date: Option<&str>) -> std::io::Result<Vec<LogEntry>> {
    let log_path = if let Some(d) = date {
        PathBuf::from(log_dir).join(format!("wftpg-{}.log", d))
    } else {
        PathBuf::from(log_dir).join(format!("wftpg-{}.log", 
            Utc::now().format("%Y-%m-%d")))
    };
    
    if !log_path.exists() {
        return Ok(Vec::new());
    }
    
    let file = File::open(log_path)?;
    let reader = BufReader::new(file);
    let mut entries = Vec::new();
    
    for line in reader.lines() {
        if let Ok(line) = line {
            if let Ok(entry) = serde_json::from_str::<LogEntry>(&line) {
                entries.push(entry);
            }
        }
    }
    
    Ok(entries)
}
