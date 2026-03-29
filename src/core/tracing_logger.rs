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
use tracing_subscriber::{fmt, layer::{Layer, SubscriberExt}, Registry};
use tracing_subscriber::filter::{Targets, LevelFilter};

/// 初始化全局日志系统
/// 
/// # Arguments
/// * `log_dir` - 日志目录
/// * `log_level` - 日志级别 (debug, info, warn, error)
/// * `max_log_size` - 单个日志文件最大大小（字节）
/// * `max_log_files` - 最大保留的日志文件数
/// * `enable_json` - 是否使用 JSON 格式输出
/// 
/// # Returns
/// * `Result<()>` - 成功或失败
pub fn init_tracing(
    log_dir: &str,
    log_level: &str,
    _max_log_size: u64,
    _max_log_files: usize,
    enable_json: bool,
) -> Result<()> {
    // 创建日志目录
    let log_path = Path::new(log_dir);
    std::fs::create_dir_all(log_path)?;
    
    // 解析日志级别
    let filter = parse_log_level(log_level);
    
    // 设置程序日志文件轮转
    let program_appender = rolling::daily(log_dir, "wftpg");
    let (non_blocking_program, _guard_program) = non_blocking(program_appender);
    
    // 设置文件操作审计日志文件轮转
    let audit_appender = rolling::daily(log_dir, "file-ops");
    let (non_blocking_audit, _guard_audit) = non_blocking(audit_appender);
    
    // 程序日志层（JSON 格式）
    let program_layer = if enable_json {
        fmt::layer()
            .with_writer(non_blocking_program)
            .with_ansi(false)
            .with_target(true)
            .with_thread_ids(false)
            .with_thread_names(false)
            .with_line_number(true)
            .json()
    } else {
        fmt::layer()
            .with_writer(non_blocking_program)
            .with_ansi(false)
            .with_target(true)
            .with_thread_ids(false)
            .with_thread_names(false)
            .with_line_number(true)
            .pretty()
    };
    
    // 文件操作审计日志层（始终使用 JSON 格式，便于解析）
    let audit_layer = fmt::layer()
        .with_writer(non_blocking_audit)
        .with_ansi(false)
        .with_target(false)
        .with_thread_ids(false)
        .with_thread_names(false)
        .with_line_number(false)
        .json();
    
    // 控制台日志层（带颜色的人类可读格式）
    let console_layer = fmt::layer()
        .with_writer(std::io::stdout)
        .with_ansi(true)
        .with_target(false)
        .with_thread_ids(false)
        .with_thread_names(false)
        .with_line_number(false)
        .pretty();
    
    // 为审计日志添加过滤器，只记录 file_ops target 的日志
    let audit_filter = Targets::new()
        .with_target("file_ops", LevelFilter::INFO);
    
    // 为程序日志添加过滤器，排除 file_ops target
    let program_filter = Targets::new()
        .with_target("file_ops", LevelFilter::OFF);
    
    // 合并所有层
    let subscriber = Registry::default()
        .with(filter.clone())
        .with(console_layer)
        .with(program_layer.with_filter(program_filter))
        .with(audit_layer.with_filter(audit_filter));
    
    // 设置全局订阅者
    tracing::subscriber::set_global_default(subscriber)?;
    
    Ok(())
}

/// 解析日志级别字符串为 Targets 过滤器
fn parse_log_level(level: &str) -> Targets {
    let level = level.to_lowercase();
    let directive = match level.as_str() {
        "trace" => "trace",
        "debug" => "debug",
        "info" => "info",
        "warn" | "warning" => "warn",
        "error" => "error",
        _ => "info", // 默认级别
    };
    
    // 设置 wftpg crate 的日志级别
    format!("wftpg={},russh={}", directive, directive)
        .parse()
        .unwrap_or_else(|_| Targets::new())
}

/// 简化版初始化（用于快速测试）
pub fn init_simple() -> Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_parse_log_level() {
        assert!(parse_log_level("debug").to_string().contains("debug"));
        assert!(parse_log_level("info").to_string().contains("info"));
        assert!(parse_log_level("warn").to_string().contains("warn"));
        assert!(parse_log_level("error").to_string().contains("error"));
        assert!(parse_log_level("invalid").to_string().contains("info"));
    }
}
