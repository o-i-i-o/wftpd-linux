//! WFTPD gRPC 契约（由 proto/wftpd.proto 生成）
//!
//! 后端（wftpd）实现 [`ControlServer`]，前端（wftp-gui）使用 [`ControlClient`]，
//! 通过 UDS 套接字通信。

#![allow(clippy::module_name_repetitions)]

pub mod convert;

pub mod wftpd {
    pub mod v1 {
        tonic::include_proto!("wftpd.v1");
    }
}

pub use wftpd::v1::{
    ConfigReply, CreateUserDirectoryRequest, EnsureUserDirectoriesRequest, FileOpLogEntry,
    FileOpLogsReply, GetConfigRequest, GetFileOpLogContentRequest, GetFileOpLogFilesRequest,
    GetInitialStateRequest, GetLogFileContentRequest, GetLogFilesRequest, GetRecentLogsRequest,
    GetStatusRequest, GetUsersRequest, InitialStateReply, LogEntry, LogEvent, LogFileInfo,
    LogFilesReply, LogsReply, OpReply, SaveConfigRequest, SaveLogConfigRequest, SaveUsersRequest,
    ServiceSelector, SetupDirectoryPermissionsRequest, StatusReply, UsersReply, WatchLogsRequest,
    WriteAuditLogRequest, control_client::ControlClient, control_server::Control,
    control_server::ControlServer,
};
