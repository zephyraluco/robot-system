//! journald 适配层（见架构文档 §4.4）。
//!
//! **journald 保存原始日志**；常驻服务只从中提取错误事件摘要与游标，不重复写入日志正文。

use serde_json::Value;

use crate::error::Result;
use crate::system;

/// 一条日志记录。
#[derive(Debug, Clone)]
pub struct LogEntry {
    /// 时间戳（Unix 秒）。
    pub timestamp: i64,
    /// syslog 优先级（0=emerg … 7=debug）。
    pub priority: u8,
    /// 日志正文。
    pub message: String,
    /// 产生日志的 PID。
    pub pid: Option<u32>,
    /// 日志标识。
    pub identifier: Option<String>,
    /// journal 游标。
    pub cursor: Option<String>,
}

/// 查询指定 unit 的日志。
///
/// * `since`：起始时间（Unix 秒），转换为 journalctl 的 `@<epoch>` 写法。
/// * `cursor`：仅返回该游标之后的记录（`--after-cursor`）。
/// * `priority`：最高优先级过滤，例如 `warning` 表示 warning 及更严重。
/// * `lines`：最多返回条数。
pub fn query(
    unit: &str,
    since: Option<i64>,
    cursor: Option<&str>,
    priority: Option<&str>,
    lines: usize,
) -> Result<Vec<LogEntry>> {
    let lines_arg = lines.to_string();
    let since_arg = since.map(|epoch| format!("@{epoch}"));

    let mut args: Vec<&str> = vec!["-u", unit, "--no-pager", "-o", "json", "-n", &lines_arg];
    if let Some(since_arg) = since_arg.as_deref() {
        args.push("--since");
        args.push(since_arg);
    }
    if let Some(cursor) = cursor {
        args.push("--after-cursor");
        args.push(cursor);
    }
    if let Some(priority) = priority {
        args.push("-p");
        args.push(priority);
    }

    let out = system::run_checked("journalctl", &args)?;
    out.lines().map(parse_json_line).collect()
}

/// 解析一行 journald JSON 输出。
pub fn parse_json_line(line: &str) -> Result<LogEntry> {
    let value: Value = serde_json::from_str(line)?;
    Ok(LogEntry {
        timestamp: value
            .get("__REALTIME_TIMESTAMP")
            .and_then(json_scalar)
            .and_then(|raw| raw.parse::<i64>().ok())
            .map_or(0, |micros| micros / 1_000_000),
        priority: value
            .get("PRIORITY")
            .and_then(json_scalar)
            .and_then(|raw| raw.parse::<u8>().ok())
            .unwrap_or(6),
        message: value.get("MESSAGE").map_or_else(String::new, json_message),
        pid: value
            .get("_PID")
            .and_then(json_scalar)
            .and_then(|raw| raw.parse::<u32>().ok()),
        identifier: value
            .get("SYSLOG_IDENTIFIER")
            .or_else(|| value.get("_COMM"))
            .and_then(json_scalar),
        cursor: value.get("__CURSOR").and_then(json_scalar),
    })
}

fn json_scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn json_message(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
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
    fn parses_json_line_with_string_message() {
        let line = r#"{"__REALTIME_TIMESTAMP":"1700000000000000","PRIORITY":"3",
            "MESSAGE":"boom","_PID":"42","SYSLOG_IDENTIFIER":"robot-lidar","__CURSOR":"s=1"}"#;
        let entry = parse_json_line(line).expect("可解析");
        assert_eq!(entry.timestamp, 1_700_000_000);
        assert_eq!(entry.priority, 3);
        assert_eq!(entry.message, "boom");
        assert_eq!(entry.pid, Some(42));
        assert_eq!(entry.cursor.as_deref(), Some("s=1"));
    }

    #[test]
    fn parses_byte_array_message() {
        let entry = parse_json_line(r#"{"MESSAGE":[111,107]}"#).expect("可解析");
        assert_eq!(entry.message, "ok");
    }

    #[test]
    fn defaults_priority_to_info() {
        let entry = parse_json_line(r#"{"MESSAGE":"x"}"#).expect("可解析");
        assert_eq!(entry.priority, 6);
        assert_eq!(entry.timestamp, 0);
    }
}
