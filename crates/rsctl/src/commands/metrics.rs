//! `rsctl metrics ...`：资源指标查询（见架构文档 附录 B）。
//!
//! 指标只由常驻服务采集并写入 SQLite；数据库不可用时给出明确提示而不报错（见 §6）。

use serde::Serialize;

use crate::app::Context;
use crate::cli::MetricsArgs;
use crate::db::MetricSample;
use crate::error::Result;
use crate::output::{emit, fmt_bytes, fmt_time};
use crate::state::StateStore;
use crate::system::systemd;

/// 指标查询报告。
#[derive(Debug, Serialize)]
struct MetricsReport {
    /// 指标历史是否可用。
    available: bool,
    /// 提示信息。
    note: Option<String>,
    /// 采样点。
    samples: Vec<MetricSample>,
}

/// 执行指标查询。
pub fn run(ctx: &Context, args: MetricsArgs) -> Result<()> {
    systemd::validate_unit_name(&args.unit)?;
    let store = StateStore::open(&ctx.paths)?;
    let samples = store.metrics_for_service(&args.unit, args.limit)?;

    let note = if !store.available() {
        Some(format!(
            "状态数据库不可用（{}）；指标需由常驻服务采集。",
            store.path().display()
        ))
    } else if samples.is_empty() {
        Some("暂无采样数据（常驻服务可能未运行，或该服务尚未被采集）。".to_string())
    } else {
        None
    };

    let report = MetricsReport {
        available: store.available(),
        note,
        samples,
    };
    emit(ctx.format, &report, |r| print_metrics(&args.unit, r))
}

fn print_metrics(unit: &str, report: &MetricsReport) {
    if let Some(note) = &report.note {
        println!("提示: {note}");
    }
    if report.samples.is_empty() {
        println!("服务 {unit} 没有可用的指标采样");
        return;
    }
    println!("服务 {unit} 的资源指标:");
    println!(
        "{:<20} {:>8} {:>12} {:>8}",
        "采样时间", "CPU%", "内存", "线程"
    );
    for sample in &report.samples {
        println!(
            "{:<20} {:>8.1} {:>12} {:>8}",
            fmt_time(Some(sample.sampled_at)),
            sample.cpu_percent,
            fmt_bytes(sample.memory_bytes),
            sample.thread_count
        );
    }
}
