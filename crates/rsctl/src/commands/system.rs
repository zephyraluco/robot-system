//! `rsctl system ...`：统一 target 与系统管理状态（见架构文档 §2、附录 B）。

use serde::Serialize;

use crate::app::Context;
use crate::cli::SystemCommand;
use crate::error::Result;
use crate::output::{dash, emit};
use crate::service::ServiceManager;
use crate::state::StateStore;

/// 系统管理状态汇总。
#[derive(Debug, Serialize)]
struct SystemReport {
    /// 受管 target 名称。
    target: String,
    /// target 加载状态。
    target_load_state: String,
    /// target 活动状态。
    target_active_state: String,
    /// target 启用状态。
    target_unit_file_state: String,
    /// 目标 target 直接依赖的服务数（来源为 systemd 的依赖关系）。
    target_services: usize,
    /// systemd 中确实存在（LoadState 非 not-found）的服务数。
    loaded_services: usize,
    /// 活动服务数。
    active_services: usize,
    /// 失败服务数。
    failed_services: usize,
    /// 状态数据库是否可用。
    state_db_available: bool,
    /// 状态数据库路径。
    state_db_path: String,
}

/// 执行系统子命令。
pub fn run(ctx: &Context, command: SystemCommand) -> Result<()> {
    match command {
        SystemCommand::Status => status(ctx),
    }
}

fn status(ctx: &Context) -> Result<()> {
    let manager = ServiceManager::new(ctx.paths.clone(), ctx.config.clone());
    let target = manager.target_status()?;
    let services = manager.list_managed()?;
    let target_units = manager.target_units()?;
    let store = StateStore::open(&ctx.paths)?;

    let report = SystemReport {
        target: manager.managed_target().to_string(),
        target_load_state: target.load_state,
        target_active_state: target.active_state,
        target_unit_file_state: target.unit_file_state,
        target_services: target_units.len(),
        loaded_services: services
            .iter()
            .filter(|s| !s.load_state.is_empty() && s.load_state != "not-found")
            .count(),
        active_services: services
            .iter()
            .filter(|s| s.active_state == "active")
            .count(),
        failed_services: services
            .iter()
            .filter(|s| s.active_state == "failed")
            .count(),
        state_db_available: store.available(),
        state_db_path: store.path().display().to_string(),
    };

    emit(ctx.format, &report, |r| {
        println!("受管 target: {}", r.target);
        println!(
            "target 状态: {} ({})，启用状态 {}",
            dash(&r.target_active_state),
            dash(&r.target_load_state),
            dash(&r.target_unit_file_state)
        );
        println!(
            "target 依赖 {} 个服务，其中已加载 {} 个；活动 {}，失败 {}",
            r.target_services, r.loaded_services, r.active_services, r.failed_services
        );
        if r.state_db_available {
            println!("状态数据库: 可用（{}）", r.state_db_path);
        } else {
            println!(
                "状态数据库: 不可用（{} 不存在；运行历史、指标与错误事件需常驻服务采集）",
                r.state_db_path
            );
        }
    })
}
