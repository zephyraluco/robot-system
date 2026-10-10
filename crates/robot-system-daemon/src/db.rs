//! 状态数据库写入与结构迁移（见架构文档 §5.1、§6.3）。
//!
//! 常驻服务是数据库**唯一的写入者**；`rsctl` 只读，因此不存在两个程序之间的写入竞争。
//! 需要在常驻服务内部控制的，只有采集、事件写入与清理任务之间的并发——本模块通过把
//! 连接包在互斥锁中对外提供同步接口来解决。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::{Error, Result};

/// 一条运行实例记录。
#[derive(Debug, Clone)]
pub struct RunRow {
    /// 运行实例 UUID。
    pub run_id: String,
    /// 服务名。
    pub service_name: String,
    /// 进程 PID。
    pub pid: i64,
    /// 内核进程启动时钟值（用于区分 PID 复用，见 §5.4）。
    pub proc_start_ticks: i64,
    /// 启动时间（Unix 秒）。
    pub started_at: i64,
}

/// 一次清理操作的统计结果。
#[derive(Debug, Default, Clone, Copy)]
pub struct PruneStats {
    /// 删除的指标采样数。
    pub metrics: usize,
    /// 删除的运行实例数。
    pub runs: usize,
    /// 删除的事件数。
    pub events: usize,
}

/// 数据保留策略（见 §6.5）。
#[derive(Debug, Clone, Copy)]
pub struct RetentionPolicy {
    /// 指标保留天数。
    pub metrics_days: u64,
    /// 运行实例保留天数。
    pub runs_days: u64,
    /// 事件保留天数。
    pub events_days: u64,
}

/// 状态数据库。
pub struct Database {
    conn: Connection,
}

impl std::fmt::Debug for Database {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Database").finish_non_exhaustive()
    }
}

impl Database {
    /// 打开（必要时创建）数据库，并设置 WAL、外键约束与忙等待超时。
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        // WAL 模式与外键约束是本系统明确要求的设置（见 §6）。
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        Ok(Self { conn })
    }

    /// 应用 `migrations/` 下尚未执行的迁移脚本，返回本次应用的版本号。
    ///
    /// 每个迁移在独立事务中执行，保证不会出现“结构改了一半”的中间态。
    pub fn migrate(&mut self, dir: &Path) -> Result<Vec<i64>> {
        let migrations = load_migrations(dir)?;
        let applied = self.applied_versions()?;

        let mut newly = Vec::new();
        for migration in migrations {
            if applied.contains(&migration.version) {
                continue;
            }
            let tx = self.conn.transaction()?;
            tx.execute_batch(&migration.sql).map_err(|err| {
                Error::Migration(format!(
                    "迁移 {}（{}）执行失败：{err}",
                    migration.version, migration.name
                ))
            })?;
            tx.execute(
                "INSERT INTO schema_migrations (version, name, applied_at) VALUES (?1, ?2, ?3)",
                params![migration.version, migration.name, now()],
            )?;
            tx.commit()?;
            newly.push(migration.version);
        }
        Ok(newly)
    }

    /// 已应用的迁移版本集合；`schema_migrations` 尚不存在时返回空集合。
    fn applied_versions(&self) -> Result<BTreeSet<i64>> {
        // 注意：表尚不存在时，`prepare` 本身就会失败，因此不能只依赖查询阶段的错误处理。
        let mut stmt = match self.conn.prepare("SELECT version FROM schema_migrations") {
            Ok(stmt) => stmt,
            Err(err) if is_missing_table(&err) => return Ok(BTreeSet::new()),
            Err(err) => return Err(err.into()),
        };
        let rows = stmt.query_map([], |row| row.get::<_, i64>(0))?;
        Ok(rows.collect::<rusqlite::Result<BTreeSet<i64>>>()?)
    }

    /// 记录一个新的运行实例，返回其 `run_id`。
    pub fn open_run(
        &self,
        service: &str,
        pid: u32,
        start_ticks: u64,
        started_at: i64,
    ) -> Result<String> {
        let run_id = uuid::Uuid::new_v4().to_string();
        self.conn.execute(
            "INSERT INTO process_runs (run_id, service_name, pid, proc_start_ticks, started_at, result) \
             VALUES (?1, ?2, ?3, ?4, ?5, 'running')",
            params![run_id, service, i64::from(pid), start_ticks as i64, started_at],
        )?;
        Ok(run_id)
    }

    /// 某服务当前处于 `running` 状态的运行实例。
    pub fn open_run_for_service(&self, service: &str) -> Result<Option<RunRow>> {
        Ok(self
            .conn
            .query_row(
                "SELECT run_id, service_name, pid, proc_start_ticks, started_at \
                 FROM process_runs WHERE service_name = ?1 AND result = 'running' \
                 ORDER BY started_at DESC LIMIT 1",
                params![service],
                map_run,
            )
            .optional()?)
    }

    /// 所有处于 `running` 状态的运行实例。
    pub fn running_runs(&self) -> Result<Vec<RunRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT run_id, service_name, pid, proc_start_ticks, started_at \
             FROM process_runs WHERE result = 'running'",
        )?;
        let rows = stmt.query_map([], map_run)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 结束一个运行实例。
    ///
    /// `exit_code` / `exit_signal` 在无法确认时传 `None`——**不伪造退出原因**（见 §6.1）。
    pub fn close_run(
        &self,
        run_id: &str,
        ended_at: i64,
        exit_code: Option<i64>,
        exit_signal: Option<i64>,
        result: &str,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE process_runs SET ended_at = ?2, exit_code = ?3, exit_signal = ?4, \
             result = ?5, runtime_ms = (?2 - started_at) * 1000 WHERE run_id = ?1",
            params![run_id, ended_at, exit_code, exit_signal, result],
        )?;
        Ok(())
    }

    /// 写入一条资源指标采样。
    pub fn insert_metric(
        &self,
        run_id: &str,
        sampled_at: i64,
        cpu_percent: f64,
        memory_bytes: i64,
        thread_count: i64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO process_metrics (run_id, sampled_at, cpu_percent, memory_bytes, thread_count) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![run_id, sampled_at, cpu_percent, memory_bytes, thread_count],
        )?;
        Ok(())
    }

    /// 写入一条运行类错误事件。
    #[allow(clippy::too_many_arguments)]
    pub fn insert_event(
        &self,
        event_type: &str,
        severity: &str,
        object_type: &str,
        object_id: &str,
        run_id: Option<&str>,
        message: &str,
        details_json: Option<&str>,
        occurred_at: i64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO error_events (event_type, severity, object_type, object_id, run_id, \
             message, details_json, occurred_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                event_type,
                severity,
                object_type,
                object_id,
                run_id,
                message,
                details_json,
                occurred_at
            ],
        )?;
        Ok(())
    }

    /// 判断某服务最近是否已经记录过指定类型的事件（用于事件去抖）。
    pub fn has_recent_event(&self, event_type: &str, object_id: &str, since: i64) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM error_events \
             WHERE event_type = ?1 AND object_id = ?2 AND occurred_at >= ?3",
            params![event_type, object_id, since],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// 某对象最近记录过的事件关联的 journal 游标。
    ///
    /// 日志采集据此实现增量读取，避免重复导入同一条日志（见 §5.5）。
    pub fn latest_cursor(&self, object_id: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT journal_cursor FROM error_events \
                 WHERE object_id = ?1 AND journal_cursor IS NOT NULL \
                 ORDER BY occurred_at DESC LIMIT 1",
                params![object_id],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// 按保留策略清理历史数据。
    pub fn prune(&self, policy: &RetentionPolicy) -> Result<PruneStats> {
        let now = now();
        let mut stats = PruneStats::default();

        let metrics_cutoff = now - days_to_secs(policy.metrics_days);
        stats.metrics += self.delete_batched(
            "DELETE FROM process_metrics WHERE rowid IN \
             (SELECT rowid FROM process_metrics WHERE sampled_at < ?1 LIMIT 1000)",
            metrics_cutoff,
        )?;

        // 只清理已结束的运行实例；`running` 的记录由采集循环负责收尾。
        let runs_cutoff = now - days_to_secs(policy.runs_days);
        stats.runs += self.delete_batched(
            "DELETE FROM process_runs WHERE rowid IN \
             (SELECT rowid FROM process_runs WHERE result <> 'running' AND started_at < ?1 LIMIT 1000)",
            runs_cutoff,
        )?;

        let events_cutoff = now - days_to_secs(policy.events_days);
        stats.events += self.delete_batched(
            "DELETE FROM error_events WHERE rowid IN \
             (SELECT rowid FROM error_events WHERE occurred_at < ?1 LIMIT 1000)",
            events_cutoff,
        )?;

        Ok(stats)
    }

    /// 分批删除，避免长时间持锁（见 §6.5）。
    fn delete_batched(&self, sql: &str, cutoff: i64) -> Result<usize> {
        let mut total = 0;
        loop {
            let deleted = self.conn.execute(sql, params![cutoff])?;
            total += deleted;
            if deleted == 0 {
                break;
            }
        }
        Ok(total)
    }
}

/// 一个待应用的迁移脚本。
#[derive(Debug, Clone)]
struct Migration {
    version: i64,
    name: String,
    sql: String,
}

/// 读取并排序迁移脚本；目录不存在时返回空列表。
fn load_migrations(dir: &Path) -> Result<Vec<Migration>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Err(Error::Migration(format!(
                "迁移目录不存在：{}（应随程序安装在 /opt/robot-system/migrations）",
                dir.display()
            )));
        }
        Err(err) => return Err(Error::Io(err)),
    };

    let mut migrations = Vec::new();
    for entry in entries {
        let path: PathBuf = entry?.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("sql") {
            continue;
        }
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string();
        let Some(version) = parse_version(&file_name) else {
            return Err(Error::Migration(format!(
                "迁移文件名不符合 `<版本>_<说明>.sql` 约定：{file_name}"
            )));
        };
        let sql = std::fs::read_to_string(&path)?;
        migrations.push(Migration {
            version,
            name: file_name,
            sql,
        });
    }
    migrations.sort_by_key(|migration| migration.version);
    Ok(migrations)
}

/// 从迁移文件名解析版本号（例如 `0002_add_runtime_indexes.sql` → `2`）。
fn parse_version(file_name: &str) -> Option<i64> {
    let prefix = file_name.split(['_', '.']).next()?;
    prefix.parse::<i64>().ok()
}

/// 判断错误是否为“表不存在”（首次运行时的预期情况）。
fn is_missing_table(err: &rusqlite::Error) -> bool {
    matches!(err, rusqlite::Error::SqliteFailure(_, Some(message)) if message.contains("no such table"))
}

fn map_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<RunRow> {
    Ok(RunRow {
        run_id: row.get(0)?,
        service_name: row.get(1)?,
        pid: row.get(2)?,
        proc_start_ticks: row.get(3)?,
        started_at: row.get(4)?,
    })
}

fn days_to_secs(days: u64) -> i64 {
    i64::try_from(days)
        .unwrap_or(i64::MAX / 86_400)
        .saturating_mul(86_400)
}

/// 当前 Unix 时间（秒）。
pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

#[cfg(test)]
impl Database {
    /// 测试辅助：访问底层连接，用于断言写入结果。
    pub(crate) fn raw_conn(&self) -> &Connection {
        &self.conn
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 仓库中的迁移目录（编译期位置 → 运行期路径）。
    fn repo_migrations_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("migrations")
    }

    fn open_db() -> (tempfile::TempDir, Database) {
        let tmp = tempfile::tempdir().expect("临时目录可用");
        let mut db = Database::open(&tmp.path().join("state.db")).expect("打开数据库");
        db.migrate(&repo_migrations_dir()).expect("迁移成功");
        (tmp, db)
    }

    #[test]
    fn migrations_are_idempotent() {
        let tmp = tempfile::tempdir().expect("临时目录可用");
        let mut db = Database::open(&tmp.path().join("state.db")).expect("打开数据库");

        let first = db.migrate(&repo_migrations_dir()).expect("首次迁移");
        assert_eq!(first, vec![1, 2]);
        let second = db.migrate(&repo_migrations_dir()).expect("再次迁移");
        assert!(second.is_empty(), "重复迁移不应重复执行");
    }

    #[test]
    fn run_lifecycle_records_runtime() {
        let (_tmp, db) = open_db();
        let run_id = db
            .open_run("robot-lidar.service", 4242, 12345, 1_700_000_000)
            .expect("创建运行实例");

        let open = db
            .open_run_for_service("robot-lidar.service")
            .expect("查询成功")
            .expect("存在运行实例");
        assert_eq!(open.run_id, run_id);
        assert_eq!(open.pid, 4242);
        assert_eq!(db.running_runs().expect("查询成功").len(), 1);

        db.close_run(&run_id, 1_700_000_060, Some(1), None, "failed")
            .expect("结束运行实例");
        assert!(
            db.open_run_for_service("robot-lidar.service")
                .expect("查询成功")
                .is_none()
        );

        let (result, runtime): (String, i64) = db
            .conn
            .query_row(
                "SELECT result, runtime_ms FROM process_runs WHERE run_id = ?1",
                params![run_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("查询成功");
        assert_eq!(result, "failed");
        assert_eq!(runtime, 60_000);
    }

    #[test]
    fn unknown_results_do_not_fake_exit_codes() {
        let (_tmp, db) = open_db();
        let run_id = db
            .open_run("robot-camera.service", 7, 1, 100)
            .expect("创建");
        db.close_run(&run_id, 200, None, None, "unknown")
            .expect("结束");

        let exit_code: Option<i64> = db
            .conn
            .query_row(
                "SELECT exit_code FROM process_runs WHERE run_id = ?1",
                params![run_id],
                |row| row.get(0),
            )
            .expect("查询成功");
        assert!(exit_code.is_none(), "无法确认时不写入退出码");
    }

    #[test]
    fn events_and_dedup_query() {
        let (_tmp, db) = open_db();
        db.insert_event(
            "process_exited_abnormally",
            "error",
            "process",
            "robot-lidar.service",
            None,
            "exit 1",
            Some("{\"code\":1}"),
            1_700_000_000,
        )
        .expect("写入事件");

        assert!(
            db.has_recent_event(
                "process_exited_abnormally",
                "robot-lidar.service",
                1_699_999_000
            )
            .expect("查询成功")
        );
        assert!(
            !db.has_recent_event(
                "process_exited_abnormally",
                "robot-lidar.service",
                1_800_000_000
            )
            .expect("查询成功")
        );
    }

    #[test]
    fn prune_removes_only_outdated_rows() {
        let (_tmp, db) = open_db();
        let old = db.open_run("s.service", 1, 1, 1_000).expect("创建");
        db.close_run(&old, 2_000, Some(0), None, "exited")
            .expect("结束");
        let fresh = db.open_run("s.service", 2, 2, now()).expect("创建");

        db.insert_metric(&old, 1_000, 1.0, 1024, 1)
            .expect("写入指标");
        db.insert_event("t", "info", "process", "s.service", None, "m", None, 1_000)
            .expect("写入事件");

        let stats = db
            .prune(&RetentionPolicy {
                metrics_days: 1,
                runs_days: 1,
                events_days: 1,
            })
            .expect("清理成功");
        assert_eq!(stats.metrics, 1);
        assert_eq!(stats.runs, 1);
        assert_eq!(stats.events, 1);

        // 仍在运行的实例不会被清理。
        assert!(
            db.open_run_for_service("s.service")
                .expect("查询成功")
                .is_some()
        );
        assert_eq!(
            db.open_run_for_service("s.service")
                .expect("查询成功")
                .unwrap()
                .run_id,
            fresh
        );
    }

    #[test]
    fn parse_version_reads_leading_number() {
        assert_eq!(parse_version("0001_initial.sql"), Some(1));
        assert_eq!(parse_version("0012_x_y.sql"), Some(12));
        assert_eq!(parse_version("abc.sql"), None);
    }

    #[test]
    fn missing_migrations_dir_is_reported() {
        let tmp = tempfile::tempdir().expect("临时目录可用");
        let mut db = Database::open(&tmp.path().join("state.db")).expect("打开数据库");
        assert!(matches!(
            db.migrate(Path::new("/nonexistent/migrations")),
            Err(Error::Migration(_))
        ));
    }
}
