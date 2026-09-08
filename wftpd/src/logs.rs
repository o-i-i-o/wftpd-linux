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
/// 扩展名比较大小写不敏感；前缀保持大小写敏感（由本程序生成，恒定小写）。
pub fn list_log_files(log_dir: &str, prefix: &str) -> Vec<LogFileEntry> {
    let Ok(entries) = std::fs::read_dir(log_dir) else {
        return Vec::new();
    };

    let mut files: Vec<LogFileEntry> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                    n.starts_with(prefix)
                        && p.extension()
                            .is_some_and(|ext| ext.eq_ignore_ascii_case("log"))
                })
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

    // ---- 文件级读取 ----

    #[test]
    fn list_log_files_filters_prefix_and_sorts_desc() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "wftpg.2026-09-01.log",
            "wftpg.2026-09-05.log",
            "wftpg.2026-09-03.log",
            "file-ops.2026-09-05.log", // 不同前缀，应被过滤
            "wftpg.2026-09-02.txt",    // 非日志扩展名，应被过滤
        ] {
            std::fs::write(dir.path().join(name), "x").unwrap();
        }

        let files = list_log_files(&dir.path().to_string_lossy(), "wftpg");
        let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["wftpg.2026-09-05", "wftpg.2026-09-03", "wftpg.2026-09-01"]
        );
        assert!(files[0].path.ends_with("wftpg.2026-09-05.log"));
    }

    #[test]
    fn list_log_files_missing_dir_returns_empty() {
        assert!(list_log_files("/nonexistent/wftpd/logs", "wftpg").is_empty());
    }

    #[test]
    fn read_program_log_returns_last_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wftpg.2026-09-08.log");
        let mut content = String::new();
        for i in 0..5 {
            use std::fmt::Write as _;
            let _ = writeln!(content, "2026-09-08T10:0{i}:00+08:00  INFO wftpd: line-{i}");
        }
        std::fs::write(&path, content).unwrap();

        let entries = read_program_log(&path.to_string_lossy(), 3);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].message, "line-2", "最早的一条应为 line-2");
        assert_eq!(entries[2].message, "line-4");

        // count 超过文件行数时返回全部
        assert_eq!(read_program_log(&path.to_string_lossy(), 100).len(), 5);
    }

    #[test]
    fn read_program_log_missing_file_returns_empty() {
        assert!(read_program_log("/nonexistent/wftpd/app.log", 10).is_empty());
    }

    #[test]
    fn read_file_op_log_parses_json_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file-ops.2026-09-08.log");
        std::fs::write(
            &path,
            concat!(
                r#"{"timestamp":"t1","fields":{"username":"u1","client_ip":"ip","operation":"UPLOAD","file_path":"/a","file_size":1,"protocol":"FTP","success":true,"message":"ok"}}"#,
                "\n",
                r#"{"timestamp":"t2","fields":{"username":"u2","client_ip":"ip","operation":"DELETE","file_path":"/b","file_size":0,"protocol":"FTP","success":false,"message":"nope"}}"#,
                "\n",
                "not-json-line\n",
            ),
        )
        .unwrap();

        let entries = read_file_op_log(&path.to_string_lossy(), 10);
        assert_eq!(entries.len(), 2, "无法解析的行应被跳过");
        assert_eq!(entries[0].operation, "UPLOAD");
        assert_eq!(entries[1].operation, "DELETE");
        assert!(!entries[1].success);
    }

    #[test]
    fn ensure_path_in_dir_accepts_inside_rejects_outside() {
        let dir = tempfile::tempdir().unwrap();
        let inside = dir.path().join("app.log");
        std::fs::write(&inside, "x").unwrap();
        let outside = std::env::temp_dir().join("wftpd-outside-test.log");
        std::fs::write(&outside, "x").unwrap();

        assert!(
            ensure_path_in_dir(&inside.to_string_lossy(), &dir.path().to_string_lossy()).is_ok()
        );
        assert!(
            ensure_path_in_dir(&outside.to_string_lossy(), &dir.path().to_string_lossy()).is_err(),
            "目录之外的路径应被拒绝"
        );
        let _ = std::fs::remove_file(&outside);
    }
}
