//! 输出辅助：统一处理“人类可读文本”与“机器可读 JSON”两种模式。

use serde::Serialize;

use crate::error::Result;

/// 输出模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// 面向运维人员的文本输出。
    Human,
    /// 面向脚本与上层程序的 JSON 输出。
    Json,
}

impl Format {
    /// 根据 CLI 全局 `--json` 开关构造。
    #[must_use]
    pub fn from_flag(json: bool) -> Self {
        if json { Self::Json } else { Self::Human }
    }
}

/// 按当前模式输出一个值：JSON 模式打印序列化结果，否则调用 `human` 渲染文本。
pub fn emit<T, F>(format: Format, value: &T, human: F) -> Result<()>
where
    T: Serialize,
    F: FnOnce(&T),
{
    match format {
        Format::Json => println!("{}", serde_json::to_string_pretty(value)?),
        Format::Human => human(value),
    }
    Ok(())
}

/// 将空字符串渲染为 `-`，避免表格出现空白列。
#[must_use]
pub fn dash(value: &str) -> &str {
    if value.is_empty() { "-" } else { value }
}

/// 将 Unix 秒格式化为本地时间字符串；`None` 或非法值返回 `-`。
#[must_use]
pub fn fmt_time(ts: Option<i64>) -> String {
    match ts.and_then(|v| chrono::DateTime::from_timestamp(v, 0)) {
        Some(dt) => dt
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M:%S")
            .to_string(),
        None => "-".to_string(),
    }
}

/// 将毫秒时长格式化为人类可读形式。
#[must_use]
pub fn fmt_duration_ms(ms: Option<i64>) -> String {
    match ms {
        None => "-".to_string(),
        Some(v) if v < 1000 => format!("{v}ms"),
        Some(v) if v < 60_000 => format!("{:.1}s", v as f64 / 1000.0),
        Some(v) if v < 3_600_000 => format!("{:.1}m", v as f64 / 60_000.0),
        Some(v) => format!("{:.1}h", v as f64 / 3_600_000.0),
    }
}

/// 将字节数格式化为人类可读形式。
#[must_use]
pub fn fmt_bytes(bytes: i64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    let b = bytes as f64;
    if b < KIB {
        format!("{bytes}B")
    } else if b < MIB {
        format!("{:.1}KiB", b / KIB)
    } else if b < GIB {
        format!("{:.1}MiB", b / MIB)
    } else {
        format!("{:.2}GiB", b / GIB)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_formatting_covers_units() {
        assert_eq!(fmt_duration_ms(Some(500)), "500ms");
        assert_eq!(fmt_duration_ms(Some(1500)), "1.5s");
        assert_eq!(fmt_duration_ms(None), "-");
    }

    #[test]
    fn bytes_formatting_covers_units() {
        assert_eq!(fmt_bytes(512), "512B");
        assert_eq!(fmt_bytes(2048), "2.0KiB");
        assert_eq!(fmt_bytes(5 * 1024 * 1024), "5.0MiB");
    }

    #[test]
    fn dash_replaces_empty_values() {
        assert_eq!(dash(""), "-");
        assert_eq!(dash("enabled"), "enabled");
    }
}
