//! Tracing 日志系统初始化模块
//!
//! 提供基于 tracing 的日志系统，支持：
//! - 动态日志级别过滤（通过配置文件，reload::Layer）
//! - JSON 格式输出（便于机器解析）
//! - 文件轮转（通过 tracing-appender）
//! - 控制台输出（带颜色和时间戳）
//! - 分离程序日志和文件操作审计日志
//! - 内存环形缓冲 + 广播通道（[`LogBuffer`]），供后端 gRPC 接口
//!   向前端提供 GetRecentLogs / WatchLogs 能力

use anyhow::Result;
use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use tracing_appender::{non_blocking, rolling};
use tracing_subscriber::filter::{LevelFilter, Targets};
use tracing_subscriber::{
    Registry,
    field::Visit,
    fmt::{self, format::FmtSpan, time::ChronoLocal},
    layer::{Layer, SubscriberExt},
    reload::{self, Handle},
};

use crate::file_logger::LogEntryJson;

static RELOAD_HANDLE: OnceLock<Handle<Targets, Registry>> = OnceLock::new();

/// 内存日志环形缓冲，同时作为 tracing Layer 挂载到全局 subscriber。
///
/// 前端通过 gRPC 读取最近日志（`recent()`）或订阅实时日志（`subscribe()`）。
#[derive(Clone)]
pub struct LogBuffer {
    inner: Arc<LogBufferInner>,
}

struct LogBufferInner {
    entries: StdMutex<VecDeque<LogEntryJson>>,
    capacity: usize,
    tx: tokio::sync::broadcast::Sender<LogEntryJson>,
}

impl LogBuffer {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = tokio::sync::broadcast::channel(capacity.max(16));
        LogBuffer {
            inner: Arc::new(LogBufferInner {
                entries: StdMutex::new(VecDeque::with_capacity(capacity.min(1024))),
                capacity,
                tx,
            }),
        }
    }

    pub fn push(&self, entry: LogEntryJson) {
        {
            let mut entries = self.inner.entries.lock().unwrap();
            if entries.len() >= self.inner.capacity {
                entries.pop_front();
            }
            entries.push_back(entry.clone());
        }
        // 没有订阅者时 send 返回 Err，忽略即可
        let _ = self.inner.tx.send(entry);
    }

    /// 最近 `count` 条日志，按时间正序返回
    pub fn recent(&self, count: usize) -> Vec<LogEntryJson> {
        let entries = self.inner.entries.lock().unwrap();
        let skip = entries.len().saturating_sub(count);
        entries.iter().skip(skip).cloned().collect()
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<LogEntryJson> {
        self.inner.tx.subscribe()
    }
}

/// 从 tracing 事件字段中提取 message / client_ip / username / action
struct FieldCollector {
    message: String,
    client_ip: Option<String>,
    username: Option<String>,
    action: Option<String>,
}

impl Visit for FieldCollector {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        match field.name() {
            "message" => self.message = format!("{:?}", value),
            "client_ip" => self.client_ip = Some(format!("{:?}", value)),
            "username" => self.username = Some(format!("{:?}", value)),
            "action" => self.action = Some(format!("{:?}", value)),
            _ => {}
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        match field.name() {
            "message" => self.message = value.to_string(),
            "client_ip" => self.client_ip = Some(value.to_string()),
            "username" => self.username = Some(value.to_string()),
            "action" => self.action = Some(value.to_string()),
            _ => {}
        }
    }
}

impl<S> Layer<S> for LogBuffer
where
    S: tracing::Subscriber,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut collector = FieldCollector {
            message: String::new(),
            client_ip: None,
            username: None,
            action: None,
        };
        event.record(&mut collector);

        let metadata = event.metadata();
        self.push(LogEntryJson {
            timestamp: chrono::Local::now().to_rfc3339(),
            level: metadata.level().to_string(),
            source: metadata.target().to_string(),
            message: collector.message,
            client_ip: collector.client_ip,
            username: collector.username,
            action: collector.action,
        });
    }
}

pub fn init_tracing(
    log_dir: &str,
    log_level: &str,
    max_log_files: usize,
    enable_json: bool,
) -> Result<LogBuffer> {
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

    let program_layer =
        program_layer.with_filter(filter.clone().with_target("file_ops", LevelFilter::OFF));

    let audit_layer = fmt::layer()
        .with_writer(non_blocking_audit)
        .with_ansi(false)
        .with_target(false)
        .with_thread_ids(false)
        .with_thread_names(false)
        .with_line_number(false)
        .with_timer(ChronoLocal::rfc_3339())
        .json()
        .with_filter(Targets::new().with_target("file_ops", LevelFilter::INFO));

    let console_layer = fmt::layer()
        .with_writer(std::io::stdout)
        .with_ansi(true)
        .with_target(false)
        .with_thread_ids(false)
        .with_thread_names(false)
        .with_line_number(false)
        .with_timer(ChronoLocal::rfc_3339())
        .with_filter(filter.clone().with_target("file_ops", LevelFilter::OFF));

    // 内存缓冲层：与程序日志同过滤规则，排除 file_ops 审计日志
    let log_buffer = LogBuffer::new(2000);
    let memory_layer = log_buffer
        .clone()
        .with_filter(filter.clone().with_target("file_ops", LevelFilter::OFF));

    let subscriber = Registry::default()
        .with(reload_filter)
        .with(console_layer)
        .with(program_layer)
        .with(audit_layer)
        .with(memory_layer);

    tracing::subscriber::set_global_default(subscriber)?;

    let _ = RELOAD_HANDLE.set(reload_handle);

    std::mem::forget(guard_program);
    std::mem::forget(guard_audit);

    Ok(log_buffer)
}

fn level_to_filter(level: &str) -> LevelFilter {
    match level.to_lowercase().as_str() {
        "trace" => LevelFilter::TRACE,
        "debug" => LevelFilter::DEBUG,
        "info" => LevelFilter::INFO,
        "warn" | "warning" => LevelFilter::WARN,
        "error" => LevelFilter::ERROR,
        "off" => LevelFilter::OFF,
        _ => LevelFilter::INFO,
    }
}

fn parse_log_level(level: &str) -> Targets {
    Targets::new()
        .with_default(level_to_filter(level))
        .with_target("russh", LevelFilter::INFO)
        .with_target("russh-keys", LevelFilter::WARN)
        .with_target("tonic", LevelFilter::INFO)
        .with_target("h2", LevelFilter::WARN)
        .with_target("tokio", LevelFilter::WARN)
        .with_target("runtime", LevelFilter::WARN)
}

pub fn set_log_level(level: &str) -> Result<()> {
    let filter = parse_log_level(level);

    if let Some(handle) = RELOAD_HANDLE.get() {
        handle.modify(|old_filter| {
            *old_filter = filter;
        })?;
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
        assert_eq!(level_to_filter("debug"), LevelFilter::DEBUG);
        assert_eq!(level_to_filter("info"), LevelFilter::INFO);
        assert_eq!(level_to_filter("warn"), LevelFilter::WARN);
        assert_eq!(level_to_filter("warning"), LevelFilter::WARN);
        assert_eq!(level_to_filter("error"), LevelFilter::ERROR);
        assert_eq!(level_to_filter("off"), LevelFilter::OFF);
        // 未知级别回退到 INFO
        assert_eq!(level_to_filter("invalid"), LevelFilter::INFO);
    }

    #[test]
    fn test_log_buffer_ring() {
        let buffer = LogBuffer::new(4);
        for i in 0..6 {
            buffer.push(LogEntryJson {
                timestamp: format!("t{}", i),
                level: "INFO".into(),
                source: "test".into(),
                message: format!("m{}", i),
                client_ip: None,
                username: None,
                action: None,
            });
        }
        let recent = buffer.recent(10);
        assert_eq!(recent.len(), 4);
        assert_eq!(recent[0].timestamp, "t2");
        assert_eq!(recent[3].timestamp, "t5");
    }
}
