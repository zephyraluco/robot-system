//! `robot-system-daemon`：robot-system 常驻服务。
//!
//! 该程序与前台 CLI `rsctl` 是**两个相互独立的程序**（见架构文档 §1）：
//! 不共享 Rust 库、不通过 IPC 通信，只共享 SQLite 数据库结构以及 systemd、journald、
//! `/proc` 等系统设施。
//!
//! 常驻服务只负责三类持续性后台工作：
//! 1. 进程监听与资源指标采集；
//! 2. 日志记录与运行类错误事件整理；
//! 3. HTTP 请求任务入口（当前仅占位）。
//!
//! 它**不执行任何软件包或服务变更操作**，也**不接受任何命令行参数**：路径与行为完全由
//! 配置文件、systemd unit 与环境变量决定（见 §12）。

mod config;
mod daemon;
mod db;
mod error;
mod event;
mod http_task;
mod logs;
mod monitor;
mod paths;
mod system;
mod target;

use std::process::ExitCode;
use std::sync::Arc;

use tokio::signal::unix::{SignalKind, signal};
use tracing::{debug, error, info, warn};
use tracing_subscriber::EnvFilter;

use crate::config::Config;
use crate::daemon::Daemon;
use crate::paths::Paths;

/// 历史数据清理的周期（以采集周期为单位，约每小时一次）。
const PRUNE_INTERVAL_TICKS: u64 = 720;

#[tokio::main]
async fn main() -> ExitCode {
    // 路径由 ROBOT_SYSTEM_PREFIX（或标准系统路径）与配置文件决定。
    let paths = Paths::detect();
    let config_path = paths.config_file();
    let config = match Config::load(&config_path) {
        Ok(config) => config,
        Err(err) => {
            eprintln!("错误：{err}");
            return ExitCode::FAILURE;
        }
    };

    // 日志守卫必须在整个进程期间存活，否则异步写入线程会被提前结束。
    let _log_guard = init_tracing(&paths);
    info!(
        version = env!("CARGO_PKG_VERSION"),
        config = %config_path.display(),
        "robot-system-daemon 启动"
    );

    let migrations_dir = paths.migrations_dir();

    let daemon = match Daemon::new(paths.clone(), config.clone(), &migrations_dir) {
        Ok(daemon) => Arc::new(daemon),
        Err(err) => {
            error!(%err, "初始化失败");
            return ExitCode::FAILURE;
        }
    };

    if let Err(err) = daemon.validate_environment() {
        error!(%err, "运行环境校验失败");
        return ExitCode::FAILURE;
    }

    // §14.1：常驻服务启动时只对账运行实例，不涉及软件包或部署任务。
    if let Err(err) = daemon.reconcile() {
        error!(%err, "运行实例对账失败");
        return ExitCode::FAILURE;
    }

    if config.http_enabled {
        let listen = config.http_listen.clone();
        info!(%listen, "启动 HTTP 请求任务入口（占位，见架构文档 §10）");
        tokio::spawn(async move {
            if let Err(err) = http_task::serve(&listen).await {
                error!(%err, "HTTP 占位入口退出");
            }
        });
    } else {
        info!("HTTP 请求任务入口未启用（占位）");
    }

    run_collection_loop(daemon, &config).await
}

/// 采集主循环：按配置周期执行采集，并定期清理历史数据。
async fn run_collection_loop(daemon: Arc<Daemon>, config: &Config) -> ExitCode {
    let mut interval = tokio::time::interval(config.sample_interval());
    let mut ticks: u64 = 0;

    loop {
        tokio::select! {
            _ = interval.tick() => {
                ticks += 1;

                let task = Arc::clone(&daemon);
                match tokio::task::spawn_blocking(move || task.tick()).await {
                    Ok(Ok(summary)) => debug!(
                        services = summary.services,
                        opened = summary.opened,
                        closed = summary.closed,
                        metrics = summary.metrics,
                        events = summary.events,
                        "采集完成"
                    ),
                    Ok(Err(err)) => error!(%err, "采集循环出错"),
                    Err(err) => error!(%err, "采集任务异常退出"),
                }

                if ticks.is_multiple_of(PRUNE_INTERVAL_TICKS) {
                    let task = Arc::clone(&daemon);
                    match tokio::task::spawn_blocking(move || task.prune()).await {
                        Ok(Ok(_)) => {}
                        Ok(Err(err)) => warn!(%err, "历史数据清理失败"),
                        Err(err) => warn!(%err, "清理任务异常退出"),
                    }
                }
            }
            _ = shutdown_signal() => {
                info!("收到停止信号，正在退出");
                break;
            }
        }
    }

    ExitCode::SUCCESS
}

/// 等待 SIGINT 或 SIGTERM。
async fn shutdown_signal() {
    let mut terminate = match signal(SignalKind::terminate()) {
        Ok(stream) => stream,
        Err(err) => {
            warn!(%err, "无法注册 SIGTERM 处理，仅响应 Ctrl+C");
            let _ = tokio::signal::ctrl_c().await;
            return;
        }
    };

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = terminate.recv() => {}
    }
}

/// 初始化日志。
///
/// 常驻服务把自身日志写入 `/var/log/robot-system/daemon.log`（见 §12）。日志标识由
/// systemd unit 的 `SyslogIdentifier` 提供，与业务服务日志区分开（见 §5.5）。
fn init_tracing(paths: &Paths) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    match std::fs::create_dir_all(&paths.log) {
        Ok(()) => {
            let appender = tracing_appender::rolling::never(&paths.log, "daemon.log");
            let (writer, guard) = tracing_appender::non_blocking(appender);
            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_writer(writer)
                .init();
            Some(guard)
        }
        Err(err) => {
            eprintln!(
                "警告：无法创建日志目录 {}：{err}，改为输出到标准输出",
                paths.log.display()
            );
            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_writer(std::io::stdout)
                .init();
            None
        }
    }
}
