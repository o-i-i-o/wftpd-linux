//! 前后端通信：基于 gRPC(tonic over UDS) 的客户端封装。
//!
//! GTK 主线程不宜直接运行异步运行时，这里提供一个进程级共享的 tokio
//! Runtime，每个调用在该运行时上阻塞执行单次 RPC（与旧 IPC 行为一致，
//! UI 侧已在独立线程中调用这些函数）。

pub mod client;

pub use client::{
    ServerStatus, connect_error_hint, get_file_log_file_content, get_log_file_content, get_logs,
    get_status, restart_service, start_service, stop_service, write_audit_log, write_config,
    write_users,
};
