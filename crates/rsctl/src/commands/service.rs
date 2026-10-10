//! `rsctl service ...`：systemd 服务生命周期管理（见架构文档 附录 B）。
//!
//! 受管变更（start/stop/restart/enable/disable）统一使用应用级文件锁串行化（见 §5.7）。

use std::time::Duration;

use crate::app::Context;
use crate::cli::ServiceCommand;
use crate::error::Result;
use crate::lock::ManagedLock;
use crate::output::{dash, emit};
use crate::service::ServiceManager;
use crate::system::command;
use crate::system::systemd::ServiceStatus;

/// 执行服务子命令。
pub fn run(ctx: &Context, command: ServiceCommand) -> Result<()> {
    match command {
        ServiceCommand::List { all } => list(ctx, all),
        ServiceCommand::Status { unit } => status(ctx, &unit),
        ServiceCommand::Start { unit } => {
            mutate(ctx, "service start", &unit, ServiceManager::start)
        }
        ServiceCommand::Stop { unit } => mutate(ctx, "service stop", &unit, ServiceManager::stop),
        ServiceCommand::Restart { unit } => {
            mutate(ctx, "service restart", &unit, ServiceManager::restart)
        }
        ServiceCommand::Enable { unit } => {
            mutate(ctx, "service enable", &unit, ServiceManager::enable)
        }
        ServiceCommand::Disable { unit } => {
            mutate(ctx, "service disable", &unit, ServiceManager::disable)
        }
    }
}

fn list(ctx: &Context, all: bool) -> Result<()> {
    let manager = ServiceManager::new(ctx.paths.clone(), ctx.config.clone());
    if all {
        let units = manager.list_all()?;
        return emit(ctx.format, &units, |items| {
            println!(
                "{:<40} {:<10} {:<10} {:<12} 描述",
                "单元", "加载", "活动", "子状态"
            );
            for entry in items {
                println!(
                    "{:<40} {:<10} {:<10} {:<12} {}",
                    entry.unit,
                    entry.load_state,
                    entry.active_state,
                    entry.sub_state,
                    entry.description
                );
            }
        });
    }

    let services = manager.list_managed()?;
    emit(ctx.format, &services, |items| {
        if items.is_empty() {
            println!(
                "受管 target {} 当前没有依赖任何服务（业务服务需以 WantedBy={} 声明归属并 enable）",
                manager.managed_target(),
                manager.managed_target()
            );
            return;
        }
        println!(
            "{:<36} {:<10} {:<10} {:<12} {:<8} 归属软件包",
            "单元", "启用", "活动", "子状态", "PID"
        );
        for svc in items {
            println!(
                "{:<36} {:<10} {:<10} {:<12} {:<8} {}",
                svc.unit,
                if svc.enabled() { "yes" } else { "no" },
                svc.active_state,
                svc.sub_state,
                svc.main_pid,
                if svc.owners.is_empty() {
                    "-".to_string()
                } else {
                    svc.owners.join(", ")
                }
            );
        }
    })
}

fn status(ctx: &Context, unit: &str) -> Result<()> {
    let manager = ServiceManager::new(ctx.paths.clone(), ctx.config.clone());
    let status = manager.status(unit)?;
    emit(ctx.format, &status, |st| print_status(unit, st))
}

/// 受管变更：校验权限、获取应用级锁、执行、回报最新状态。
fn mutate<F>(ctx: &Context, action: &str, unit: &str, operation: F) -> Result<()>
where
    F: FnOnce(&ServiceManager, &str) -> Result<()>,
{
    command::require_root(action)?;
    let _lock = ManagedLock::acquire(
        &ctx.paths.lock_file(),
        Duration::from_secs(ctx.config.lock_timeout_secs),
    )?;

    let manager = ServiceManager::new(ctx.paths.clone(), ctx.config.clone());
    operation(&manager, unit)?;

    let status = manager.status(unit)?;
    emit(ctx.format, &status, |st| print_status(unit, st))
}

/// 打印服务状态。
pub fn print_status(unit: &str, status: &ServiceStatus) {
    println!("单元: {unit}");
    println!("加载状态: {}", dash(&status.load_state));
    println!(
        "活动状态: {} ({})",
        dash(&status.active_state),
        dash(&status.sub_state)
    );
    println!("启用状态: {}", dash(&status.unit_file_state));
    println!("主进程 PID: {}", status.main_pid);
    println!("结果: {}", dash(&status.result));
    println!("重启次数: {}", status.n_restarts);
    if status.exec_main_status != 0 || status.result != "success" {
        println!(
            "主进程退出: status={} code={}",
            status.exec_main_status, status.exec_main_code
        );
    }
}
