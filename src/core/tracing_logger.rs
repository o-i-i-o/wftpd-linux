//! Tracing 日志系统初始化模块
//! 
//! 提供基于 tracing 的日志系统，支持：
//! - 动态日志级别过滤（通过配置文件）
//! - JSON 格式输出（便于机器解析）
//! - 文件轮转（通过 tracing-appender）
//! - 控制台输出（带颜色和时间戳）
//! - 分离程序日志和文件操作审计日志

use anyhow::Result;
use std::path::Path;
use tracing_appender::{non_blocking, rolling};
use tracing_subscriber::{
    fmt::{self, format::FmtSpan, time::ChronoLocal},
    layer::{Layer, SubscriberExt},
    reload::{self, Handle},
    Registry,
};
use tracing_subscriber::filter::{LevelFilter, Targets};

static mut RELOAD_HANDLE: Option<Handle<Targets, Registry>> = None;

pub fn init_tracing(
    log_dir: &str,
    log_level: &str,
    max_log_files: usize,
    enable_json: bool,
) -> Result<()> {
    let log_path = Path::new(log_dir);
    std::fs::create_dir_all(log_path)?;

    let filter = parse_log_level(log_level);

    let program_appender = rolling::RollingFileAppender::builder()
        .rotation(rolling::Rotation::DAILY)
        .filename_prefix("wftpg")
        .filename_suffix("log")
        .max_log_files(max_log_files)
        .build(log_dir)?;

    let (non_blocking_program, guard_program) = non_blocking(program_appender);

    let audit_appender = rolling::RollingFileAppender::builder()
        .rotation(rolling::Rotation::DAILY)
        .filename_prefix("file-ops")
        .filename_suffix("log")
        .max_log_files(max_log_files)
        .build(log_dir)?;

    let (non_blocking_audit, guard_audit) = non_blocking(audit_appender);

    let (reload_filter, reload_handle) = reload::Layer::new(filter.clone());

    let program_layer = if enable_json {
        fmt::layer()
            .with_writer(non_blocking_program)
            .with_ansi(false)
            .with_target(true)
            .with_thread_ids(false)
            .with_thread_names(false)
            .with_line_number(true)
            .with_span_events(FmtSpan::CLOSE)
            .with_timer(ChronoLocal::rfc_3339())
            .json()
            .boxed()
    } else {
        fmt::layer()
            .with_writer(non_blocking_program)
            .with_ansi(false)
            .with_target(true)
            .with_thread_ids(false)
            .with_thread_names(false)
            .with_line_number(true)
            .with_span_events(FmtSpan::CLOSE)
            .with_timer(ChronoLocal::rfc_3339())
            .boxed()
    };

    let program_layer = program_layer.with_filter(
        filter
            .with_target("file_ops", LevelFilter::OFF)
    );

    let audit_layer = fmt::layer()
        .with_writer(non_blocking_audit)
        .with_ansi(false)
        .with_target(false)
        .with_thread_ids(false)
        .with_thread_names(false)
        .with_line_number(false)
        .with_timer(ChronoLocal::rfc_3339())
        .json()
        .with_filter(
            Targets::new()
                .with_target("file_ops", LevelFilter::INFO)
        );

    let console_layer = fmt::layer()
        .with_writer(std::io::stdout)
        .with_ansi(true)
        .with_target(false)
        .with_thread_ids(false)
        .with_thread_names(false)
        .with_line_number(false)
        .with_timer(ChronoLocal::rfc_3339())
        .with_filter(
            Targets::new()
                .with_target("file_ops", LevelFilter::OFF)
        );

    let subscriber = Registry::default()
        .with(reload_filter)
        .with(console_layer)
        .with(program_layer)
        .with(audit_layer);

    tracing::subscriber::set_global_default(subscriber)?;

    unsafe {
        RELOAD_HANDLE = Some(reload_handle);
    }

    std::mem::forget(guard_program);
    std::mem::forget(guard_audit);

    Ok(())
}

fn parse_log_level(level: &str) -> Targets {
    let level = level.to_lowercase();
    let level_filter = match level.as_str() {
        "trace" => LevelFilter::TRACE,
        "debug" => LevelFilter::DEBUG,
        "info" => LevelFilter::INFO,
        "warn" | "warning" => LevelFilter::WARN,
        "error" => LevelFilter::ERROR,
        "off" => LevelFilter::OFF,
        _ => LevelFilter::INFO,
    };

    Targets::new()
        .with_default(level_filter)
        .with_target("russh", LevelFilter::INFO)
        .with_target("russh-keys", LevelFilter::WARN)
        .with_target("tokio", LevelFilter::WARN)
        .with_target("runtime", LevelFilter::WARN)
}

pub fn set_log_level(level: &str) -> Result<()> {
    let filter = parse_log_level(level);
    
    unsafe {
        if let Some(ref handle) = RELOAD_HANDLE {
            handle.modify(|old_filter| {
                *old_filter = filter;
            })?;
        }
    }
    
    Ok(())
}

pub fn init_simple() -> Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .with_line_number(false)
        .init();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_parse_log_level() {
        let filter = parse_log_level("debug");
        assert!(format!("{:?}", filter).contains("debug"));
        
        let filter = parse_log_level("info");
        assert!(format!("{:?}", filter).contains("info"));
        
        let filter = parse_log_level("warn");
        assert!(format!("{:?}", filter).contains("warn"));
        
        let filter = parse_log_level("error");
        assert!(format!("{:?}", filter).contains("error"));
        
        let filter = parse_log_level("invalid");
        assert!(format!("{:?}", filter).contains("info"));
    }
}
