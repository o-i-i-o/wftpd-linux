//! 日志文件读取与解析：供 gRPC 日志接口使用。
//!
//! 程序日志由 tracing-appender `写出（enable_json=true` 时为 JSON 行，
//! 否则为文本行），文件操作审计日志恒为 JSON 行。解析均为尽力而为：
//! 单行解析失败时跳过该行，不影响其余内容。

use std::io::BufRead;
use std::path::Path;

use wftpd_common::{FileLogEntryJson, LogEntryJson, LogFileEntry};

/// 列出目录下带指定前缀的日志文件（新→旧），例如 prefix="wftpg" 匹配 wftpg.2026-09-05.log
///
/// 文件名由 tracing-appender 按 prefix/suffix 生成，恒为小写，因此扩展名比较保持大小写敏感
#[allow(clippy::case_sensitive_file_extension_comparisons)]
pub fn list_log_files(log_dir: &str, prefix: &str) -> Vec<LogFileEntry> {
    let Ok(entries) = std::fs::read_dir(log_dir) else {
        return Vec::new();
    };

    let mut files: Vec<LogFileEntry> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(prefix) && n.ends_with(".log"))
        })
        .map(|p| LogFileEntry {
            name: p
                .file_stem()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string(),
            path: p.to_string_lossy().into_owned(),
        })
        .collect();

    files.sort_by(|a, b| b.name.cmp(&a.name));
    files
}

/// 读取文件最后 `count` 行
fn tail_lines(path: &str, count: usize) -> Vec<String> {
    let Ok(file) = std::fs::File::open(path) else {
        return Vec::new();
    };

    let mut lines = Vec::new();
    for line in std::io::BufReader::new(file).lines().map_while(Result::ok) {
        lines.push(line);
        if lines.len() > count * 2 {
            let drain_to = lines.len() - count;
            lines.drain(..drain_to);
        }
    }

    if lines.len() > count {
        lines.split_off(lines.len() - count)
    } else {
        lines
    }
}

fn opt_to_string(v: serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => Some(s),
        serde_json::Value::Null => None,
        other => Some(other.to_string()),
    }
}

/// 解析一行程序日志（JSON 或文本格式）
fn parse_program_line(line: &str) -> Option<LogEntryJson> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    if let Ok(v) = serde_json::from_str::<serde_json::Value>(line)
        && v.is_object()
    {
        return Some(LogEntryJson {
            timestamp: opt_to_string(v.get("timestamp").cloned().unwrap_or_default())
                .unwrap_or_default(),
            level: opt_to_string(v.get("level").cloned().unwrap_or_default())
                .unwrap_or_else(|| "INFO".into()),
            source: opt_to_string(v.get("target").cloned().unwrap_or_default()).unwrap_or_default(),
            message: v
                .get("fields")
                .and_then(|f| f.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or_default()
                .to_string(),
            client_ip: v
                .get("fields")
                .and_then(|f| f.get("client_ip"))
                .and_then(|m| m.as_str())
                .map(str::to_string),
            username: v
                .get("fields")
                .and_then(|f| f.get("username"))
                .and_then(|m| m.as_str())
                .map(str::to_string),
            action: v
                .get("fields")
                .and_then(|f| f.get("action"))
                .and_then(|m| m.as_str())
                .map(str::to_string),
        });
    }

    // 文本格式：`<rfc3339时间> <LEVEL(右对齐填充)> <target>: <message>`
    let (timestamp, remainder) = line.split_once(' ')?;
    let remainder = remainder.trim_start();
    let (level, rest) = remainder.split_once(' ')?;
    let rest = rest.trim_start();
    let (source, message) = match rest.split_once(": ") {
        Some((s, m)) => (s.to_string(), m.to_string()),
        None => (String::new(), rest.to_string()),
    };

    Some(LogEntryJson {
        timestamp: timestamp.to_string(),
        level: level.to_string(),
        source,
        message,
        client_ip: None,
        username: None,
        action: None,
    })
}

/// 解析一行文件操作审计日志（JSON 格式）
fn parse_file_op_line(line: &str) -> Option<FileLogEntryJson> {
    let v = serde_json::from_str::<serde_json::Value>(line.trim()).ok()?;
    let field = |name: &str| -> String {
        v.get("fields")
            .and_then(|f| f.get(name))
            .and_then(|m| m.as_str())
            .unwrap_or_default()
            .to_string()
    };

    Some(FileLogEntryJson {
        timestamp: v
            .get("timestamp")
            .and_then(|t| t.as_str())
            .unwrap_or_default()
            .to_string(),
        username: field("username"),
        client_ip: field("client_ip"),
        operation: field("operation"),
        file_path: field("file_path"),
        file_size: v
            .get("fields")
            .and_then(|f| f.get("file_size"))
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default(),
        protocol: field("protocol"),
        success: v
            .get("fields")
            .and_then(|f| f.get("success"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or_default(),
        message: field("message"),
    })
}

/// 读取程序日志文件最后 count 条
pub fn read_program_log(path: &str, count: usize) -> Vec<LogEntryJson> {
    tail_lines(path, count)
        .iter()
        .filter_map(|l| parse_program_line(l))
        .collect()
}

/// 读取文件操作审计日志最后 count 条
pub fn read_file_op_log(path: &str, count: usize) -> Vec<FileLogEntryJson> {
    tail_lines(path, count)
        .iter()
        .filter_map(|l| parse_file_op_line(l))
        .collect()
}

/// 校验日志文件路径位于指定目录内，防止越权读取任意文件
pub fn ensure_path_in_dir(path: &str, dir: &str) -> anyhow::Result<()> {
    let canonical = std::fs::canonicalize(Path::new(path))?;
    let log_dir = std::fs::canonicalize(Path::new(dir))?;
    if !canonical.starts_with(&log_dir) {
        anyhow::bail!("path {path} is outside log directory");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_text_line() {
        let entry =
            parse_program_line("2026-09-05T10:00:00+08:00  INFO wftpd_ftp::server: FTP 服务已启动")
                .expect("parse ok");
        assert_eq!(entry.level, "INFO");
        assert_eq!(entry.source, "wftpd_ftp::server");
        assert_eq!(entry.message, "FTP 服务已启动");
    }

    #[test]
    fn test_parse_json_line() {
        let line = r#"{"timestamp":"2026-09-05T10:00:00+08:00","level":"INFO","target":"wftpd","fields":{"message":"hello","client_ip":"1.2.3.4"}}"#;
        let entry = parse_program_line(line).expect("parse ok");
        assert_eq!(entry.source, "wftpd");
        assert_eq!(entry.message, "hello");
        assert_eq!(entry.client_ip.as_deref(), Some("1.2.3.4"));
    }

    #[test]
    fn test_parse_file_op_line() {
        let line = r#"{"timestamp":"2026-09-05T10:00:00+08:00","fields":{"username":"u","client_ip":"1.2.3.4","operation":"UPLOAD","file_path":"/a","file_size":10,"protocol":"FTP","success":true,"message":"文件上传成功"}}"#;
        let entry = parse_file_op_line(line).expect("parse ok");
        assert_eq!(entry.operation, "UPLOAD");
        assert_eq!(entry.file_size, 10);
        assert!(entry.success);
    }
}
