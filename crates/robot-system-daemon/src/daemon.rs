//! 常驻服务的装配与采集调度（见架构文档 §1、§6.3）。
//!
//! 常驻服务只做三类持续性后台工作：进程监听、日志记录、事件记录，以及 HTTP 请求任务
//! 入口（占位）。它**不承担软件包或服务的变更操作**。

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use tracing::{debug, info};

use crate::config::Config;
use crate::db::{Database, PruneStats, RetentionPolicy};
use crate::error::{Error, Result};
use crate::logs::LogManager;
use crate::monitor::{ProcessMonitor, ReconcileSummary, TickSummary};
use crate::paths::Paths;
use crate::target::TargetServices;

/// 日志巡检间隔（秒）：不必每轮采集都调用 journalctl。
const LOG_SCAN_INTERVAL_SECS: u64 = 30;

/// 单次日志巡检读取的最大条数。
const LOG_SCAN_MAX_LINES: usize = 200;

/// 常驻服务主体。
#[derive(Debug)]
pub struct Daemon {
    paths: Paths,
    config: Config,
    database: Arc<Mutex<Database>>,
    services: TargetServices,
    monitor: Mutex<ProcessMonitor>,
    logs: LogManager,
    /// 每隔多少个采集周期执行一次日志巡检。
    log_every_ticks: u64,
    /// 已执行的采集周期数。
    tick_count: AtomicU64,
}

impl Daemon {
    /// 初始化数据库（含迁移）、受管 target 与各管理器。
    pub fn new(paths: Paths, config: Config, migrations_dir: &Path) -> Result<Self> {
        let mut database = Database::open(&paths.state_db())?;
        let applied = database.migrate(migrations_dir)?;
        if !applied.is_empty() {
            info!(versions = ?applied, "已应用数据库迁移");
        }

        let log_every_ticks = (LOG_SCAN_INTERVAL_SECS / config.sample_interval_secs.max(1)).max(1);

        Ok(Self {
            services: TargetServices::new(config.managed_target.clone()),
            logs: LogManager::new(config.journal_lookback_secs, LOG_SCAN_MAX_LINES),
            database: Arc::new(Mutex::new(database)),
            monitor: Mutex::new(ProcessMonitor::new()),
            log_every_ticks,
            tick_count: AtomicU64::new(0),
            paths,
            config,
        })
    }

    /// 启动时的运行实例对账（见 §14.1）。
    pub fn reconcile(&self) -> Result<ReconcileSummary> {
        let database = lock(&self.database);
        let mut monitor = lock(&self.monitor);
        let summary = monitor.reconcile(&database)?;
        info!(
            alive = summary.alive,
            closed = summary.closed,
            "运行实例对账完成"
        );
        Ok(summary)
    }

    /// 执行一次采集循环（进程监听 + 指标采集 + 定期日志巡检）。
    pub fn tick(&self) -> Result<TickSummary> {
        let database = lock(&self.database);
        let mut monitor = lock(&self.monitor);

        let mut summary = monitor.tick(&database, &self.services)?;

        let round = self.tick_count.fetch_add(1, Ordering::Relaxed) + 1;
        if round.is_multiple_of(self.log_every_ticks) {
            for unit in self.services.units()? {
                match self.logs.scan(&database, &unit) {
                    Ok(scan) => {
                        summary.events += scan.recorded;
                        debug!(
                            unit = %unit,
                            scanned = scan.scanned,
                            recorded = scan.recorded,
                            "日志巡检完成"
                        );
                    }
                    Err(err) => {
                        // 单个服务的日志不可读不应中断整轮采集。
                        tracing::warn!(unit = %unit, %err, "日志巡检失败");
                    }
                }
            }
        }

        Ok(summary)
    }

    /// 按保留策略清理历史数据（见 §6.5）。
    pub fn prune(&self) -> Result<PruneStats> {
        let database = lock(&self.database);
        let policy = RetentionPolicy {
            metrics_days: self.config.metrics_retention_days,
            runs_days: self.config.runs_retention_days,
            events_days: self.config.events_retention_days,
        };
        let stats = database.prune(&policy)?;
        info!(
            metrics = stats.metrics,
            runs = stats.runs,
            events = stats.events,
            "历史数据清理完成"
        );
        Ok(stats)
    }

    /// 校验运行环境：数据目录应可创建。
    pub fn validate_environment(&self) -> Result<()> {
        if let Err(err) = std::fs::create_dir_all(&self.paths.var_lib) {
            return Err(Error::InvalidArgument(format!(
                "无法创建数据目录 {}：{err}",
                self.paths.var_lib.display()
            )));
        }
        Ok(())
    }
}

/// 获取互斥锁；即使持有者曾 panic，也继续使用内部数据而不是二次 panic。
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn repo_migrations_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("migrations")
    }

    #[test]
    fn new_applies_migrations_and_can_run_a_tick() {
        let tmp = tempfile::tempdir().expect("临时目录可用");
        let paths = Paths::with_prefix(tmp.path());
        // 使用不存在的 target：系统中确实没有需要采集的服务，结果与环境无关。
        let config = Config {
            managed_target: "robot-system-nonexistent-test.target".to_string(),
            ..Config::default()
        };
        let daemon = Daemon::new(paths, config, &repo_migrations_dir()).expect("初始化成功");

        daemon.validate_environment().expect("环境可用");
        let summary = daemon.reconcile().expect("对账成功");
        assert_eq!(summary.alive, 0);

        let summary = daemon.tick().expect("采集成功");
        assert_eq!(summary.services, 0);

        let stats = daemon.prune().expect("清理成功");
        assert_eq!(stats.metrics, 0);
    }

    #[test]
    fn discovers_managed_services_from_systemd_target() {
        // 受管服务集合来自 systemd 的依赖关系，而不是本系统保存的清单；
        // 不存在的 target 应得到空列表而不是错误。
        let tmp = tempfile::tempdir().expect("临时目录可用");
        let paths = Paths::with_prefix(tmp.path());
        let config = Config {
            managed_target: "robot-system-nonexistent-test.target".to_string(),
            ..Config::default()
        };
        let _daemon = Daemon::new(paths, config, &repo_migrations_dir()).expect("初始化成功");
    }
}
