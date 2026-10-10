//! `rsctl error ...`：运行类错误事件查询（见架构文档 附录 B）。
//!
//! `error_events` 表只保存常驻服务产生的运行类事件（进程、服务）；软件包与部署类事件
//! 由 `rsctl` 记录在部署任务文件中（见 §6.3、§6.4）。

use serde::Serialize;

use crate::app::Context;
use crate::cli::ErrorCommand;
use crate::db::ErrorEvent;
use crate::error::Result;
use crate::output::{emit, fmt_time};
use crate::state::StateStore;

/// 错误事件查询报告。
#[derive(Debug, Serialize)]
struct EventsReport {
    /// 事件历史是否可用。
    available: bool,
    /// 提示信息。
    note: Option<String>,
    /// 事件列表。
    events: Vec<ErrorEvent>,
}

/// 执行错误事件子命令。
pub fn run(ctx: &Context, command: ErrorCommand) -> Result<()> {
    match command {
        ErrorCommand::List {
            event_type,
            service,
            limit,
        } => list(ctx, event_type.as_deref(), service.as_deref(), limit),
    }
}

fn list(ctx: &Context, event_type: Option<&str>, service: Option<&str>, limit: u32) -> Result<()> {
    let store = StateStore::open(&ctx.paths)?;
    let events = store.error_events(event_type, service, limit)?;

    let note = if !store.available() {
        Some(format!(
            "状态数据库不可用（{}）；运行类错误事件需由常驻服务采集。",
            store.path().display()
        ))
    } else if events.is_empty() {
        Some("没有匹配的错误事件".to_string())
    } else {
        None
    };

    let report = EventsReport {
        available: store.available(),
        note,
        events,
    };
    emit(ctx.format, &report, print_events)
}

fn print_events(report: &EventsReport) {
    if let Some(note) = &report.note {
        println!("提示: {note}");
    }
    if report.events.is_empty() {
        return;
    }
    println!(
        "{:<20} {:<9} {:<30} {:<28} 摘要",
        "发生时间", "级别", "事件类型", "对象"
    );
    for event in &report.events {
        println!(
            "{:<20} {:<9} {:<30} {:<28} {}",
            fmt_time(Some(event.occurred_at)),
            event.severity,
            event.event_type,
            format!("{} {}", event.object_type, event.object_id),
            event.message
        );
    }
}
