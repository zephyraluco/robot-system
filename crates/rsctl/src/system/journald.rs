//! journald 适配层（见架构文档 §4.4）。
//!
//! **journald 保存原始日志**，本模块只负责查询与解析，不把日志正文重复写入 SQLite。

use serde::Serialize;
use serde_json::Value;

use crate::error::Result;
use crate::system::command;

/// 一条日志记录。
#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    /// 时间戳（Unix 秒）。
    pub timestamp: i64,
    /// syslog 优先级（0=emerg … 7=debug）。
    pub priority: u8,
    /// 日志正文。
    pub message: String,
    /// 产生日志的 PID。
    pub pid: Option<u32>,
    /// 日志标识（`SYSLOG_IDENTIFIER`）。
    pub identifier: Option<String>,
    /// journal 游标，可用于后续精确续读。
    pub cursor: Option<String>,
}

impl LogEntry {
    /// syslog 优先级的名称。
    #[must_use]
    pub fn priority_name(&self) -> &'static str {
        match self.priority {
            0 => "emerg",
            1 => "alert",
            2 => "crit",
            3 => "err",
            4 => "warning",
            5 => "notice",
            6 => "info",
            _ => "debug",
        }
    }
}

/// 查询指定 unit 的日志。
///
/// * `since`：时间过滤，支持 journalctl 原生写法（如 `2024-01-01`、`-1h`），
///   也支持简写 `10m` / `2h` / `30s` / `1d`。
/// * `priority`：优先级过滤（如 `err`、`warning` 或数字）。
/// * `lines`：最多返回的日志条数。
pub fn query(
    unit: &str,
    since: Option<&str>,
    priority: Option<&str>,
    lines: usize,
) -> Result<Vec<LogEntry>> {
    let lines_arg = lines.to_string();
    let normalized_since = since.map(normalize_since);
    let mut args: Vec<&str> = vec!["-u", unit, "--no-pager", "-o", "json", "-n", &lines_arg];
    if let Some(since) = normalized_since.as_deref() {
        args.push("--since");
        args.push(since);
    }
    if let Some(priority) = priority {
        args.push("-p");
        args.push(priority);
    }

    let out = command::run_checked("journalctl", &args)?;
    out.lines().map(parse_json_line).collect()
}

/// 将简写时长转换为 journalctl 可识别的时间表达式。
#[must_use]
pub fn normalize_since(input: &str) -> String {
    let trimmed = input.trim();
    let (digits, unit) = trimmed.split_at(trimmed.len().saturating_sub(1));
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return trimmed.to_string();
    }
    match unit {
        "s" => format!("{digits} sec ago"),
        "m" => format!("{digits} min ago"),
        "h" => format!("{digits} hour ago"),
        "d" => format!("{digits} day ago"),
        _ => trimmed.to_string(),
    }
}

/// 解析一行 journald JSON 输出。
fn parse_json_line(line: &str) -> Result<LogEntry> {
    let value: Value = serde_json::from_str(line)?;
    Ok(LogEntry {
        timestamp: value
            .get("__REALTIME_TIMESTAMP")
            .and_then(json_scalar)
            .and_then(|s| s.parse::<i64>().ok())
            .map_or(0, |micros| micros / 1_000_000),
        priority: value
            .get("PRIORITY")
            .and_then(json_scalar)
            .and_then(|s| s.parse::<u8>().ok())
            .unwrap_or(6),
        message: value.get("MESSAGE").map_or_else(String::new, json_message),
        pid: value
            .get("_PID")
            .and_then(json_scalar)
            .and_then(|s| s.parse::<u32>().ok()),
        identifier: value
            .get("SYSLOG_IDENTIFIER")
            .or_else(|| value.get("_COMM"))
            .and_then(json_scalar),
        cursor: value.get("__CURSOR").and_then(json_scalar),
    })
}

/// journald 的字段可能是字符串，也可能是字节数组（非 UTF-8 时）。
fn json_scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// 解析 `MESSAGE` 字段，兼容字符串与字节数组两种形式。
fn json_message(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Array(items) => {
            let bytes: Vec<u8> = items
                .iter()
                .filter_map(|item| item.as_u64().and_then(|n| u8::try_from(n).ok()))
                .collect();
            String::from_utf8_lossy(&bytes).into_owned()
        }
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_shorthand_durations() {
        assert_eq!(normalize_since("10m"), "10 min ago");
        assert_eq!(normalize_since("2h"), "2 hour ago");
        assert_eq!(normalize_since("1d"), "1 day ago");
        assert_eq!(normalize_since("2024-01-01"), "2024-01-01");
        assert_eq!(normalize_since("-1h"), "-1h");
    }

    #[test]
    fn parses_string_message() {
        let line = r#"{"__REALTIME_TIMESTAMP":"1700000000000000","PRIORITY":"3",
            "MESSAGE":"boom","_PID":"42","SYSLOG_IDENTIFIER":"robot-lidar","__CURSOR":"s=abc"}"#;
        let entry = parse_json_line(line).expect("可解析");
        assert_eq!(entry.timestamp, 1_700_000_000);
        assert_eq!(entry.priority, 3);
        assert_eq!(entry.priority_name(), "err");
        assert_eq!(entry.message, "boom");
        assert_eq!(entry.pid, Some(42));
        assert_eq!(entry.identifier.as_deref(), Some("robot-lidar"));
    }

    #[test]
    fn parses_byte_array_message() {
        let line = r#"{"MESSAGE":[104,105]}"#;
        let entry = parse_json_line(line).expect("可解析");
        assert_eq!(entry.message, "hi");
        assert_eq!(entry.priority, 6);
    }

    #[test]
    fn rejects_malformed_json() {
        assert!(parse_json_line("not json").is_err());
    }
}
