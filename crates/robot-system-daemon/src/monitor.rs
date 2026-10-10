//! `ProcessMonitor`：进程运行实例追踪与资源采集（见架构文档 §4.2、§4.3、§4.4）。
//!
//! 采集循环每次执行：
//! 1. 读取受管服务的 systemd 状态；
//! 2. 按 `PID + 内核启动时钟值` 识别进程实例，服务重启时**生成新的 `run_id`**，
//!    绝不覆盖上一次记录（见 §8.2）；
//! 3. 在实例结束时写入结束时间与可确认的退出结果，不伪造退出码；
//! 4. 采集 CPU、内存、线程数指标。

use std::collections::HashMap;
use std::time::Instant;

use tracing::debug;

use crate::db::{self, Database};
use crate::error::Result;
use crate::event;
use crate::system::{procfs, systemd};
use crate::target::TargetServices;

/// 一次采集循环的统计。
#[derive(Debug, Default, Clone, Copy)]
pub struct TickSummary {
    /// 检查的服务数。
    pub services: usize,
    /// 新建的运行实例数。
    pub opened: usize,
    /// 结束的运行实例数。
    pub closed: usize,
    /// 写入的指标采样数。
    pub metrics: usize,
    /// 记录的事件数。
    pub events: usize,
}

/// 启动时对账的统计。
#[derive(Debug, Default, Clone, Copy)]
pub struct ReconcileSummary {
    /// 仍然存活、维持 `running` 的记录数。
    pub alive: usize,
    /// 进程已不存在、被标记为 `unknown` 的记录数。
    pub closed: usize,
}

/// 进程监听器。
#[derive(Debug)]
pub struct ProcessMonitor {
    /// 每秒时钟滴答数。
    ticks_per_sec: u64,
    /// 运行实例 → (上次 CPU 时钟滴答, 上次采样时刻)，用于计算 CPU 使用率。
    cpu_samples: HashMap<String, (u64, Instant)>,
    /// 服务 → 上一次观察到的 `ActiveState`，用于识别状态跃迁。
    last_active_state: HashMap<String, String>,
}

impl Default for ProcessMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessMonitor {
    /// 构造监听器。
    #[must_use]
    pub fn new() -> Self {
        Self {
            ticks_per_sec: procfs::clock_ticks(),
            cpu_samples: HashMap::new(),
            last_active_state: HashMap::new(),
        }
    }

    /// 启动时对账：核对数据库中的 `running` 记录与系统实际进程。
    ///
    /// 若进程已不存在（例如常驻服务曾重启），则把结果记为
    /// `unknown`——**不凭空推断退出原因**。
    pub fn reconcile(&mut self, database: &Database) -> Result<ReconcileSummary> {
        let mut summary = ReconcileSummary::default();

        for run in database.running_runs()? {
            let pid = u32::try_from(run.pid).ok();
            let stat = match pid {
                Some(pid) => procfs::read_stat(pid)?,
                None => None,
            };

            // 仅当 PID 与内核启动时钟值都一致时，才认为是同一个进程实例。
            let same_process = stat
                .as_ref()
                .is_some_and(|stat| stat.start_ticks as i64 == run.proc_start_ticks);

            debug!(
                run_id = %run.run_id,
                service = %run.service_name,
                started_at = run.started_at,
                pid = run.pid,
                alive = same_process,
                "运行实例对账"
            );

            if same_process {
                summary.alive += 1;
                if let Some(stat) = stat {
                    self.cpu_samples
                        .insert(run.run_id.clone(), (stat.cpu_ticks(), Instant::now()));
                }
            } else {
                database.close_run(&run.run_id, db::now(), None, None, "unknown")?;
                self.cpu_samples.remove(&run.run_id);
                summary.closed += 1;
            }
        }
        Ok(summary)
    }

    /// 执行一次采集循环。
    pub fn tick(&mut self, database: &Database, services: &TargetServices) -> Result<TickSummary> {
        let mut summary = TickSummary::default();

        for unit in services.units()? {
            systemd::validate_unit_name(&unit)?;
            let status = systemd::show(&unit)?;
            summary.services += 1;

            // target 可能已声明但 unit 尚未加载（例如依赖缺失）；此时本轮跳过。
            if !status.exists() {
                continue;
            }

            let previous = self
                .last_active_state
                .insert(unit.clone(), status.active_state.clone());
            let was_active = previous.as_deref() == Some("active");

            let open = database.open_run_for_service(&unit)?;

            if status.is_transitioning() {
                continue;
            }

            if status.is_active() {
                if status.main_pid == 0 {
                    // 无主进程：常见于 `Type=oneshot` 已执行完毕。若还有未收尾的实例，按
                    // systemd 记录收尾。
                    if let Some(run) = open {
                        let (code, signal, result) = status.exit_details();
                        database.close_run(&run.run_id, db::now(), code, signal, result)?;
                        self.cpu_samples.remove(&run.run_id);
                        summary.closed += 1;
                        if result == "failed" {
                            event::process_exited_abnormally(&unit, &run.run_id, code, signal)
                                .record(database)?;
                            summary.events += 1;
                        }
                    }
                    continue;
                }

                let pid = status.main_pid;
                let Some(stat) = procfs::read_stat(pid)? else {
                    // 进程在两次采样之间退出，下一轮循环再处理。
                    continue;
                };

                let same_instance = open.as_ref().is_some_and(|run| {
                    run.pid == i64::from(pid) && run.proc_start_ticks == stat.start_ticks as i64
                });

                let run_id = if same_instance {
                    open.as_ref()
                        .map(|run| run.run_id.clone())
                        .unwrap_or_default()
                } else {
                    // 服务（重新）启动：先为上一个未收尾的实例收尾，再生成新的 run_id。
                    if let Some(run) = &open {
                        database.close_run(&run.run_id, db::now(), None, None, "unknown")?;
                        self.cpu_samples.remove(&run.run_id);
                        summary.closed += 1;
                    }
                    let started_at = procfs::start_time_unix(pid).unwrap_or_else(db::now);
                    let run_id = database.open_run(&unit, pid, stat.start_ticks, started_at)?;
                    summary.opened += 1;
                    run_id
                };

                if self.record_metric(database, &run_id, &stat)? {
                    summary.metrics += 1;
                }
                continue;
            }

            // 非活动状态：`inactive` / `failed`。
            if let Some(run) = open {
                let (code, signal, result) = status.exit_details();
                database.close_run(&run.run_id, db::now(), code, signal, result)?;
                self.cpu_samples.remove(&run.run_id);
                summary.closed += 1;

                if result == "failed" {
                    event::process_exited_abnormally(&unit, &run.run_id, code, signal)
                        .record(database)?;
                    summary.events += 1;
                } else if result == "exited" && was_active && !status.is_oneshot() {
                    // 正常退出但服务本应持续运行（如未配置 Restart 且被外部终止）。
                    event::service_stopped_unexpectedly(&unit, &run.run_id).record(database)?;
                    summary.events += 1;
                }
            } else if status.is_failed() && previous.as_deref() != Some("failed") {
                // 从未观察到运行实例却已失败：属于启动失败。仅在“刚进入 failed”时记录一次。
                event::service_start_failed(&unit, &status).record(database)?;
                summary.events += 1;
            }
        }

        Ok(summary)
    }

    /// 写入一条指标采样；返回是否实际写入。
    fn record_metric(
        &mut self,
        database: &Database,
        run_id: &str,
        stat: &procfs::ProcStat,
    ) -> Result<bool> {
        if run_id.is_empty() {
            return Ok(false);
        }
        let ticks = stat.cpu_ticks();
        let now_instant = Instant::now();

        let cpu_percent = match self
            .cpu_samples
            .insert(run_id.to_string(), (ticks, now_instant))
        {
            Some((previous_ticks, previous_instant)) => {
                let elapsed = now_instant.duration_since(previous_instant).as_secs_f64();
                let delta = ticks.saturating_sub(previous_ticks);
                procfs::cpu_percent(delta, elapsed, self.ticks_per_sec)
            }
            // 首次采样缺少基线，记录 0 而不是猜测一个数值。
            None => 0.0,
        };

        let memory = stat
            .memory_bytes()
            .map_or(0, |bytes| i64::try_from(bytes).unwrap_or(i64::MAX));
        let threads = i64::try_from(stat.num_threads).unwrap_or(i64::MAX);
        database.insert_metric(run_id, db::now(), cpu_percent, memory, threads)?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn open_db() -> (tempfile::TempDir, Database) {
        let tmp = tempfile::tempdir().expect("临时目录可用");
        let mut db = Database::open(&tmp.path().join("state.db")).expect("打开数据库");
        let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("migrations");
        db.migrate(&migrations).expect("迁移成功");
        (tmp, db)
    }

    #[test]
    fn reconcile_closes_runs_for_dead_processes() {
        let (_tmp, db) = open_db();
        // PID 1 必然存在，但启动时钟值不匹配 → 应被视为已结束。
        let stale = db
            .open_run("robot-lidar.service", 1, 999_999_999, 1_000)
            .expect("创建");
        // 一个几乎不可能存在的 PID。
        let gone = db
            .open_run("robot-camera.service", u32::MAX, 1, 1_000)
            .expect("创建");

        let mut monitor = ProcessMonitor::new();
        let summary = monitor.reconcile(&db).expect("对账成功");
        assert_eq!(summary.closed, 2);
        assert_eq!(summary.alive, 0);
        assert!(
            db.open_run_for_service("robot-lidar.service")
                .expect("查询")
                .is_none()
        );

        let result: String = db
            .raw_conn()
            .query_row(
                "SELECT result FROM process_runs WHERE run_id = ?1",
                rusqlite::params![stale],
                |row| row.get(0),
            )
            .expect("查询成功");
        assert_eq!(result, "unknown", "无法确认退出原因时应记为 unknown");

        let result: String = db
            .raw_conn()
            .query_row(
                "SELECT result FROM process_runs WHERE run_id = ?1",
                rusqlite::params![gone],
                |row| row.get(0),
            )
            .expect("查询成功");
        assert_eq!(result, "unknown");
    }

    #[test]
    fn cpu_baseline_is_primed_by_reconcile() {
        let (_tmp, db) = open_db();
        let run_id = db.open_run("s.service", 1, 1, 1_000).expect("创建");
        // 用 PID 1 的真实启动时钟值构造一个“存活”记录。
        let ticks = procfs::read_stat(1)
            .expect("读取")
            .expect("PID 1 存在")
            .start_ticks;
        db.raw_conn()
            .execute(
                "UPDATE process_runs SET proc_start_ticks = ?2 WHERE run_id = ?1",
                rusqlite::params![run_id, ticks as i64],
            )
            .expect("更新成功");

        let mut monitor = ProcessMonitor::new();
        let summary = monitor.reconcile(&db).expect("对账成功");
        assert_eq!(summary.alive, 1);
        assert!(monitor.cpu_samples.contains_key(&run_id), "应预热 CPU 基线");
    }

    #[test]
    fn tick_with_target_without_services_does_nothing() {
        let (_tmp, db) = open_db();
        // target 不存在 → 没有任何需要监听的服务。
        let services = TargetServices::new("robot-system-nonexistent-test.target");
        let mut monitor = ProcessMonitor::new();
        let summary = monitor.tick(&db, &services).expect("采集成功");
        assert_eq!(summary.services, 0);
        assert_eq!(summary.services, 0);
    }
}
