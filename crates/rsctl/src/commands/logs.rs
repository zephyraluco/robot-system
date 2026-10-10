//! `rsctl logs ...`：服务原始日志查询（见架构文档 §4.4、附录 B）。
//!
//! 原始日志由 journald 保存，本命令直接查询 journald，不经过数据库，也不经过常驻服务。

use serde::Serialize;

use crate::app::Context;
use crate::cli::LogsArgs;
use crate::error::Result;
use crate::output::{emit, fmt_time};
use crate::system::{journald, systemd};

/// 日志查询报告。
#[derive(Debug, Serialize)]
struct LogsReport {
    /// 服务名。
    unit: String,
    /// 起始时间过滤。
    since: Option<String>,
    /// 优先级过滤。
    priority: Option<String>,
    /// 日志条目。
    entries: Vec<journald::LogEntry>,
}

/// 执行日志查询。
pub fn run(ctx: &Context, args: LogsArgs) -> Result<()> {
    systemd::validate_unit_name(&args.unit)?;
    let entries = journald::query(
        &args.unit,
        args.since.as_deref(),
        args.priority.as_deref(),
        args.lines,
    )?;

    let report = LogsReport {
        unit: args.unit,
        since: args.since,
        priority: args.priority,
        entries,
    };
    emit(ctx.format, &report, print_logs)
}

fn print_logs(report: &LogsReport) {
    if report.entries.is_empty() {
        println!("服务 {} 在指定范围内没有日志", report.unit);
        return;
    }
    for entry in &report.entries {
        let identifier = entry.identifier.as_deref().unwrap_or("-");
        let pid = entry.pid.map_or_else(|| "-".to_string(), |p| p.to_string());
        println!(
            "{} {:<7} {}[{}] {}",
            fmt_time(Some(entry.timestamp)),
            entry.priority_name(),
            identifier,
            pid,
            entry.message
        );
    }
}
