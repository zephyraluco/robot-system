//! `rsctl package ...`：软件包管理与部署任务（见架构文档 附录 B）。

use std::path::Path;

use crate::app::Context;
use crate::cli::{PackageCommand, TaskCommand};
use crate::deployment::{DeploymentManager, DeploymentTask, TaskKind, TaskState};
use crate::error::{Error, Result};
use crate::output::{emit, fmt_duration_ms, fmt_time};
use crate::package::{ManagedPackage, PackageManager};

/// 执行软件包子命令。
pub fn run(ctx: &Context, command: PackageCommand) -> Result<()> {
    match command {
        PackageCommand::List => list(ctx),
        PackageCommand::Info { name } => info(ctx, &name),
        PackageCommand::Install { deb } => install(ctx, &deb, TaskKind::Install),
        PackageCommand::Upgrade { deb } => install(ctx, &deb, TaskKind::Upgrade),
        PackageCommand::Remove { name } => remove(ctx, &name),
        PackageCommand::Task(TaskCommand::List) => task_list(ctx),
        PackageCommand::Task(TaskCommand::Show { id }) => task_show(ctx, &id),
        PackageCommand::History { name } => history(ctx, &name),
    }
}

fn list(ctx: &Context) -> Result<()> {
    let packages = PackageManager::new(ctx.paths.clone()).managed_packages()?;
    emit(ctx.format, &packages, |items| {
        if items.is_empty() {
            println!(
                "没有受管软件包（{} 下缺少服务清单）",
                ctx.paths.packages_dir().display()
            );
            return;
        }
        println!("{:<24} {:<10} {:<16} 服务", "包名", "已安装", "版本");
        for pkg in items {
            println!(
                "{:<24} {:<10} {:<16} {}",
                pkg.name,
                if pkg.installed { "yes" } else { "no" },
                pkg.installed_version.as_deref().unwrap_or("-"),
                if pkg.services.is_empty() {
                    "-".to_string()
                } else {
                    pkg.services.join(", ")
                }
            );
        }
    })
}

fn info(ctx: &Context, name: &str) -> Result<()> {
    let package = PackageManager::new(ctx.paths.clone()).package_info(name)?;
    emit(ctx.format, &package, print_package)
}

fn print_package(pkg: &ManagedPackage) {
    println!("包名: {}", pkg.name);
    println!(
        "安装状态: {}",
        if pkg.installed {
            "已安装"
        } else {
            "未安装"
        }
    );
    println!(
        "已安装版本: {}",
        pkg.installed_version.as_deref().unwrap_or("-")
    );
    println!(
        "清单声明版本: {}",
        pkg.declared_version.as_deref().unwrap_or("-")
    );
    println!("架构: {}", pkg.architecture.as_deref().unwrap_or("-"));
    println!(
        "服务: {}",
        if pkg.services.is_empty() {
            "-".to_string()
        } else {
            pkg.services.join(", ")
        }
    );
    if let Some(path) = &pkg.manifest_path {
        println!("清单文件: {path}");
    }
}

fn install(ctx: &Context, deb: &Path, kind: TaskKind) -> Result<()> {
    let manager = DeploymentManager::new(ctx.paths.clone(), ctx.config.clone())?;
    let task = manager.install(deb, kind)?;
    emit(ctx.format, &task, print_task)?;
    conclude(&task)
}

fn remove(ctx: &Context, name: &str) -> Result<()> {
    let manager = DeploymentManager::new(ctx.paths.clone(), ctx.config.clone())?;
    let task = manager.remove(name)?;
    emit(ctx.format, &task, print_task)?;
    conclude(&task)
}

fn task_list(ctx: &Context) -> Result<()> {
    let manager = DeploymentManager::new(ctx.paths.clone(), ctx.config.clone())?;
    let tasks = manager.list_tasks()?;
    emit(ctx.format, &tasks, |items| {
        if items.is_empty() {
            println!("暂无部署任务");
            return;
        }
        println!(
            "{:<38} {:<9} {:<20} {:<18} 创建时间",
            "任务 ID", "类型", "软件包", "状态"
        );
        for task in items {
            println!(
                "{:<38} {:<9} {:<20} {:<18} {}",
                task.id,
                task.kind.as_str(),
                task.package,
                task.state.as_str(),
                fmt_time(Some(task.created_at))
            );
        }
    })
}

fn task_show(ctx: &Context, id: &str) -> Result<()> {
    let manager = DeploymentManager::new(ctx.paths.clone(), ctx.config.clone())?;
    let task = manager.task(id)?;
    emit(ctx.format, &task, print_task)?;
    conclude(&task)
}

fn history(ctx: &Context, name: &str) -> Result<()> {
    let manager = DeploymentManager::new(ctx.paths.clone(), ctx.config.clone())?;
    let tasks = manager.history(name)?;
    emit(ctx.format, &tasks, |items| {
        if items.is_empty() {
            println!("软件包 {name} 暂无部署历史");
            return;
        }
        for task in items {
            println!(
                "{}  {}  {} -> {}  {}",
                fmt_time(Some(task.created_at)),
                task.kind.as_str(),
                task.from_version.as_deref().unwrap_or("-"),
                task.to_version.as_deref().unwrap_or("-"),
                task.state.as_str()
            );
        }
    })
}

/// 打印任务详情与步骤记录。
fn print_task(task: &DeploymentTask) {
    println!("任务 ID: {}", task.id);
    println!("类型: {}", task.kind.as_str());
    println!("软件包: {}", task.package);
    println!("源版本: {}", task.from_version.as_deref().unwrap_or("-"));
    println!("目标版本: {}", task.to_version.as_deref().unwrap_or("-"));
    println!("架构: {}", task.architecture.as_deref().unwrap_or("-"));
    println!("来源 DEB: {}", task.source_deb.as_deref().unwrap_or("-"));
    println!("SHA-256: {}", task.sha256.as_deref().unwrap_or("-"));
    println!("状态: {}", task.state.as_str());
    println!("操作者: {}", task.actor);
    println!("创建时间: {}", fmt_time(Some(task.created_at)));
    println!("结束时间: {}", fmt_time(task.finished_at));
    println!(
        "耗时: {}",
        fmt_duration_ms(task.finished_at.map(|end| (end - task.created_at) * 1000))
    );
    if let Some(err) = &task.error {
        println!("错误: {err}");
    }
    if let Some(recovery) = &task.recovery {
        println!("恢复: {recovery}");
    }
    println!("步骤:");
    for step in &task.steps {
        println!(
            "  [{}] {:<20} {}",
            fmt_time(Some(step.at)),
            step.state.as_str(),
            step.message
        );
    }
}

/// 根据任务终态决定命令的退出行为。
///
/// 只有 [`TaskState::Committed`] 视为成功；失败但已恢复、或需要人工恢复时返回非零退出码，
/// 避免脚本把失败的部署当成成功（见 §17.10）。
fn conclude(task: &DeploymentTask) -> Result<()> {
    match task.state {
        TaskState::Committed => Ok(()),
        TaskState::Recovered => Err(Error::Validation(format!(
            "部署失败但已恢复（任务 {}）：{}",
            task.id,
            task.recovery.as_deref().unwrap_or("已回滚")
        ))),
        TaskState::RecoveryRequired => Err(Error::Validation(format!(
            "部署失败且需要人工恢复（任务 {}）：{}",
            task.id,
            task.error.as_deref().unwrap_or("未知原因")
        ))),
        other => Err(Error::Validation(format!(
            "部署任务以未预期状态结束：{}（任务 {}）",
            other.as_str(),
            task.id
        ))),
    }
}
