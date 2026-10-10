//! `rsctl process ...`：进程运行实例查询（见架构文档 附录 B）。
//!
//! 数据来源优先为常驻服务采集的历史（SQLite）；当数据库不可用时，回退为直接查询
//! systemd 与 `/proc` 获取当前运行实例，并明确提示历史不可用（见 §6）。

use serde::Serialize;

use crate::app::Context;
use crate::cli::ProcessCommand;
use crate::error::Result;
use crate::output::{emit, fmt_time};
use crate::service::ServiceManager;
use crate::state::StateStore;
use crate::system::{procfs, systemd};

/// 单条运行实例（统一数据库与 systemd 两种来源）。
#[derive(Debug, Serialize)]
struct ProcessListItem {
    /// 运行实例 UUID（仅数据库来源有值）。
    run_id: Option<String>,
    /// 服务名。
    service_name: String,
    /// 进程 PID。
    pid: Option<i64>,
    /// 启动时间。
    started_at: Option<i64>,
    /// 运行结果。
    result: String,
    /// 数据来源：`database` 或 `systemd`。
    source: &'static str,
}

/// 运行实例列表报告。
#[derive(Debug, Serialize)]
struct ProcessListReport {
    /// 数据来源。
    source: &'static str,
    /// 提示信息。
    note: Option<String>,
    /// 列表内容。
    items: Vec<ProcessListItem>,
}

/// 运行历史报告。
#[derive(Debug, Serialize)]
struct ProcessHistoryReport {
    /// 历史数据是否可用。
    available: bool,
    /// 提示信息。
    note: Option<String>,
    /// 运行历史。
    runs: Vec<crate::db::ProcessRun>,
}

/// 执行进程子命令。
pub fn run(ctx: &Context, command: ProcessCommand) -> Result<()> {
    match command {
        ProcessCommand::List { limit } => list(ctx, limit),
        ProcessCommand::History { unit, limit } => history(ctx, &unit, limit),
    }
}

fn list(ctx: &Context, limit: u32) -> Result<()> {
    let store = StateStore::open(&ctx.paths)?;
    if store.available() {
        let mut runs = store.running_runs()?;
        runs.truncate(limit as usize);
        let items: Vec<ProcessListItem> = runs
            .into_iter()
            .map(|run| ProcessListItem {
                run_id: Some(run.run_id),
                service_name: run.service_name,
                pid: Some(run.pid),
                started_at: Some(run.started_at),
                result: run.result,
                source: "database",
            })
            .collect();
        let report = ProcessListReport {
            source: "database",
            note: None,
            items,
        };
        return emit(ctx.format, &report, print_process_list);
    }

    // 数据库不可用：回退为直接查询 systemd 与 /proc。
    let manager = ServiceManager::new(ctx.paths.clone(), ctx.config.clone());
    let mut items = Vec::new();
    for service in manager.list_managed()? {
        if service.main_pid == 0 {
            continue;
        }
        let pid = i64::from(service.main_pid);
        items.push(ProcessListItem {
            run_id: None,
            service_name: service.unit.clone(),
            pid: Some(pid),
            started_at: procfs::start_time_unix(service.main_pid),
            result: "running".to_string(),
            source: "systemd",
        });
    }
    items.truncate(limit as usize);

    let report = ProcessListReport {
        source: "systemd",
        note: Some(format!(
            "状态数据库不可用（{}）；以下为实时查询结果，无运行历史。",
            store.path().display()
        )),
        items,
    };
    emit(ctx.format, &report, print_process_list)
}

fn history(ctx: &Context, unit: &str, limit: u32) -> Result<()> {
    systemd::validate_unit_name(unit)?;
    let store = StateStore::open(&ctx.paths)?;
    let runs = store.process_runs(Some(unit), limit)?;
    let report = ProcessHistoryReport {
        available: store.available(),
        note: if store.available() {
            None
        } else {
            Some(format!("状态数据库不可用（{}）", store.path().display()))
        },
        runs,
    };
    emit(ctx.format, &report, print_process_history)
}

fn print_process_list(report: &ProcessListReport) {
    if let Some(note) = &report.note {
        println!("提示: {note}");
    }
    if report.items.is_empty() {
        println!("当前没有正在运行的服务进程");
        return;
    }
    println!(
        "{:<38} {:<28} {:<8} {:<20} 结果",
        "运行实例", "服务", "PID", "启动时间"
    );
    for item in &report.items {
        println!(
            "{:<38} {:<28} {:<8} {:<20} {}",
            item.run_id.as_deref().unwrap_or("-"),
            item.service_name,
            item.pid.map_or("-".to_string(), |p| p.to_string()),
            fmt_time(item.started_at),
            item.result
        );
    }
}

fn print_process_history(report: &ProcessHistoryReport) {
    if let Some(note) = &report.note {
        println!("提示: {note}");
    }
    if report.runs.is_empty() {
        println!("没有运行历史");
        return;
    }
    println!(
        "{:<38} {:<8} {:<20} {:<20} {:<10} 结果",
        "运行实例", "PID", "启动时间", "结束时间", "时长"
    );
    for run in &report.runs {
        println!(
            "{:<38} {:<8} {:<20} {:<20} {:<10} {}",
            run.run_id,
            run.pid,
            fmt_time(Some(run.started_at)),
            fmt_time(run.ended_at),
            crate::output::fmt_duration_ms(run.runtime_ms),
            run.result
        );
    }
}
