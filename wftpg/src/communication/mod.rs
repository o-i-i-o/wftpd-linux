pub mod protocol;
pub mod server;
pub mod client;

pub use protocol::{
    IpcRequest, IpcResponse, IpcCommand, IpcResult, LogEntryJson, LogFileEntry, FileLogEntryJson,
    SOCKET_PATH, CONFIG_PATH, USERS_PATH, AUDIT_LOG_PATH,
};
pub use server::IpcServer;
pub use client::{
    IpcClient, ServerStatus, IpcResponseWrapper, with_runtime, read_config, write_config, 
    read_users, write_users, write_audit_log, get_log_files, get_log_file_content,
    get_file_log_files, get_file_log_file_content, save_log_config, setup_directory_permissions, 
    create_user_directory, get_initial_state, ensure_user_directories, restart_service,
};
