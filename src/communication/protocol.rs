use serde::{Deserialize, Serialize};

pub const SOCKET_PATH: &str = "/run/wftpd/wftpg.sock";
pub const CONFIG_PATH: &str = "/etc/wftpg/config.toml";
pub const USERS_PATH: &str = "/etc/wftpg/users.json";
pub const AUDIT_LOG_PATH: &str = "/var/log/wftpg/audit.log";
pub const WFTPG_GROUP_NAME: &str = "wftpg";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcRequest {
    pub id: u64,
    pub command: IpcCommand,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum IpcCommand {
    ReloadConfig,
    ReloadUsers,
    GetConfig,
    SaveConfig { content: String },
    GetUsers,
    SaveUsers { content: String },
    RestartService,
    GetStatus,
    GetLogs { count: usize },
    SubscribeLogs,
    UnsubscribeLogs,
    WriteAuditLog {
        user: String,
        action: String,
        target: String,
        details: String,
    },
    ConfigExists,
    UsersExists,
    GetInitialState,
    EnsureUserDirectories,
    GetLogFiles,
    GetLogFileContent { path: String, count: usize },
    GetFileLogFiles,
    GetFileLogFileContent { path: String, count: usize },
    SaveLogConfig {
        log_dir: String,
        log_level: String,
        max_log_size: u64,
        max_log_files: usize,
        log_to_file: bool,
        log_to_gui: bool,
    },
    SetupDirectoryPermissions { path: String },
    CreateUserDirectory { path: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcResponse {
    pub id: u64,
    pub result: IpcResult,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum IpcResult {
    Success { message: String },
    Error { message: String },
    Config { content: String },
    Users { content: String },
    ConfigSaved { message: String, content: String },
    UsersSaved { message: String, content: String },
    Status { ftp_running: bool, sftp_running: bool },
    Logs { entries: Vec<LogEntryJson> },
    LogEntry { entry: LogEntryJson },
    Bool { value: bool },
    InitialState { config: String, users: String, ftp_running: bool, sftp_running: bool },
    LogFiles { files: Vec<LogFileEntry> },
    FileLogFiles { files: Vec<LogFileEntry> },
    FileLogEntries { entries: Vec<FileLogEntryJson> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogFileEntry {
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntryJson {
    pub timestamp: String,
    pub level: String,
    pub source: String,
    pub message: String,
    pub client_ip: Option<String>,
    pub username: Option<String>,
    pub action: Option<String>,
}

impl IpcRequest {
    pub fn new(id: u64, command: IpcCommand) -> Self {
        Self { id, command }
    }
}

impl IpcResponse {
    pub fn success(id: u64, message: &str) -> Self {
        Self {
            id,
            result: IpcResult::Success {
                message: message.to_string(),
            },
        }
    }

    pub fn error(id: u64, message: &str) -> Self {
        Self {
            id,
            result: IpcResult::Error {
                message: message.to_string(),
            },
        }
    }

    pub fn config(id: u64, content: String) -> Self {
        Self {
            id,
            result: IpcResult::Config { content },
        }
    }

    pub fn users(id: u64, content: String) -> Self {
        Self {
            id,
            result: IpcResult::Users { content },
        }
    }

    pub fn config_saved(id: u64, message: &str, content: String) -> Self {
        Self {
            id,
            result: IpcResult::ConfigSaved {
                message: message.to_string(),
                content,
            },
        }
    }

    pub fn users_saved(id: u64, message: &str, content: String) -> Self {
        Self {
            id,
            result: IpcResult::UsersSaved {
                message: message.to_string(),
                content,
            },
        }
    }

    pub fn status(id: u64, ftp_running: bool, sftp_running: bool) -> Self {
        Self {
            id,
            result: IpcResult::Status {
                ftp_running,
                sftp_running,
            },
        }
    }

    pub fn logs(id: u64, entries: Vec<LogEntryJson>) -> Self {
        Self {
            id,
            result: IpcResult::Logs { entries },
        }
    }

    pub fn log_entry(entry: LogEntryJson) -> Self {
        Self {
            id: 0,
            result: IpcResult::LogEntry { entry },
        }
    }

    pub fn bool_value(id: u64, value: bool) -> Self {
        Self {
            id,
            result: IpcResult::Bool { value },
        }
    }

    pub fn initial_state(id: u64, config: String, users: String, ftp_running: bool, sftp_running: bool) -> Self {
        Self {
            id,
            result: IpcResult::InitialState {
                config,
                users,
                ftp_running,
                sftp_running,
            },
        }
    }

    pub fn log_files(id: u64, files: Vec<LogFileEntry>) -> Self {
        Self {
            id,
            result: IpcResult::LogFiles { files },
        }
    }

    pub fn file_log_files(id: u64, files: Vec<LogFileEntry>) -> Self {
        Self {
            id,
            result: IpcResult::FileLogFiles { files },
        }
    }
}
