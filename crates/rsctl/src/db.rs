//! SQLite 状态数据库的**只读**访问（见架构文档 §5.1）。
//!
//! 数据库 `/var/lib/robot-system/state.db` 由常驻服务 `robot-system-daemon` 写入，
//! `rsctl` **只读**。这样两个独立程序之间不存在写入竞争。
//!
//! 数据库可能尚未创建（常驻服务从未运行）。此时 [`ReadOnlyDb::open`] 返回 `None`，
//! 调用方应回退为直接查询 systemd、journald 与 `/proc`，并明确提示历史不可用，
//! 而不是把数据缺失当成故障。

use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

use crate::error::Result;

/// 一条进程运行实例记录。
#[derive(Debug, Clone, Serialize)]
pub struct ProcessRun {
    /// 运行实例 UUID。
    pub run_id: String,
    /// systemd 服务名。
    pub service_name: String,
    /// 进程 PID。
    pub pid: i64,
    /// 内核进程启动时钟值。
    pub proc_start_ticks: i64,
    /// 启动时间（Unix 秒）。
    pub started_at: i64,
    /// 结束时间（Unix 秒）。
    pub ended_at: Option<i64>,
    /// 退出码。
    pub exit_code: Option<i64>,
    /// 终止信号。
    pub exit_signal: Option<i64>,
    /// 运行时长（毫秒）。
    pub runtime_ms: Option<i64>,
    /// 运行结果：running / exited / failed / unknown。
    pub result: String,
}

/// 一条资源指标采样。
#[derive(Debug, Clone, Serialize)]
pub struct MetricSample {
    /// 所属运行实例。
    pub run_id: String,
    /// 服务名。
    pub service_name: String,
    /// 采样时间（Unix 秒）。
    pub sampled_at: i64,
    /// CPU 使用率（百分比）。
    pub cpu_percent: f64,
    /// 内存使用量（字节）。
    pub memory_bytes: i64,
    /// 线程数。
    pub thread_count: i64,
}

/// 一条运行类错误事件。
#[derive(Debug, Clone, Serialize)]
pub struct ErrorEvent {
    /// 事件 ID。
    pub id: i64,
    /// 事件类型。
    pub event_type: String,
    /// 严重级别。
    pub severity: String,
    /// 对象类型（service / process）。
    pub object_type: String,
    /// 对象标识。
    pub object_id: String,
    /// 关联的运行实例。
    pub run_id: Option<String>,
    /// 错误摘要。
    pub message: String,
    /// 结构化详情（JSON 字符串）。
    pub details_json: Option<String>,
    /// 发生时间（Unix 秒）。
    pub occurred_at: i64,
    /// 关联的 journal 游标。
    pub journal_cursor: Option<String>,
}

/// 只读数据库句柄。
#[derive(Debug)]
pub struct ReadOnlyDb {
    conn: Connection,
}

const RUN_COLUMNS: &str = "run_id, service_name, pid, proc_start_ticks, started_at, \
ended_at, exit_code, exit_signal, runtime_ms, result";

impl ReadOnlyDb {
    /// 以只读方式打开数据库。
    ///
    /// 数据库文件不存在时返回 `Ok(None)`，表示“尚无采集数据”。
    pub fn open(path: &Path) -> Result<Option<Self>> {
        if !path.is_file() {
            return Ok(None);
        }
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(Duration::from_secs(5))?;
        Ok(Some(Self { conn }))
    }

    /// 当前处于 `running` 状态的运行实例。
    pub fn running_runs(&self) -> Result<Vec<ProcessRun>> {
        let sql = format!(
            "SELECT {RUN_COLUMNS} FROM process_runs WHERE result = 'running' ORDER BY started_at DESC"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], map_process_run)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 查询进程运行历史。
    ///
    /// `service` 为 `None` 时返回所有服务的记录。
    pub fn process_runs(&self, service: Option<&str>, limit: u32) -> Result<Vec<ProcessRun>> {
        let mut rows = Vec::new();
        match service {
            Some(service) => {
                let sql = format!(
                    "SELECT {RUN_COLUMNS} FROM process_runs WHERE service_name = ?1 \
                     ORDER BY started_at DESC LIMIT ?2"
                );
                let mut stmt = self.conn.prepare(&sql)?;
                let iter = stmt.query_map(rusqlite::params![service, limit], map_process_run)?;
                for row in iter {
                    rows.push(row?);
                }
            }
            None => {
                let sql = format!(
                    "SELECT {RUN_COLUMNS} FROM process_runs ORDER BY started_at DESC LIMIT ?1"
                );
                let mut stmt = self.conn.prepare(&sql)?;
                let iter = stmt.query_map(rusqlite::params![limit], map_process_run)?;
                for row in iter {
                    rows.push(row?);
                }
            }
        }
        Ok(rows)
    }

    /// 查询指定服务的最近指标采样（跨运行实例）。
    pub fn metrics_for_service(&self, service: &str, limit: u32) -> Result<Vec<MetricSample>> {
        let sql = "SELECT m.run_id, r.service_name, m.sampled_at, m.cpu_percent, \
                   m.memory_bytes, m.thread_count \
                   FROM process_metrics m JOIN process_runs r ON r.run_id = m.run_id \
                   WHERE r.service_name = ?1 ORDER BY m.sampled_at DESC LIMIT ?2";
        let mut stmt = self.conn.prepare(sql)?;
        let iter = stmt.query_map(rusqlite::params![service, limit], map_metric)?;
        Ok(iter.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 查询错误事件。
    pub fn error_events(
        &self,
        event_type: Option<&str>,
        object: Option<&str>,
        limit: u32,
    ) -> Result<Vec<ErrorEvent>> {
        let sql = "SELECT id, event_type, severity, object_type, object_id, run_id, \
                   message, details_json, occurred_at, journal_cursor FROM error_events \
                   WHERE (?1 IS NULL OR event_type = ?1) \
                     AND (?2 IS NULL OR object_id = ?2) \
                   ORDER BY occurred_at DESC LIMIT ?3";
        let mut stmt = self.conn.prepare(sql)?;
        let iter = stmt.query_map(
            rusqlite::params![event_type, object, limit],
            map_error_event,
        )?;
        Ok(iter.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

fn map_process_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProcessRun> {
    Ok(ProcessRun {
        run_id: row.get(0)?,
        service_name: row.get(1)?,
        pid: row.get(2)?,
        proc_start_ticks: row.get(3)?,
        started_at: row.get(4)?,
        ended_at: row.get(5)?,
        exit_code: row.get(6)?,
        exit_signal: row.get(7)?,
        runtime_ms: row.get(8)?,
        result: row.get(9)?,
    })
}

fn map_metric(row: &rusqlite::Row<'_>) -> rusqlite::Result<MetricSample> {
    Ok(MetricSample {
        run_id: row.get(0)?,
        service_name: row.get(1)?,
        sampled_at: row.get(2)?,
        cpu_percent: row.get(3)?,
        memory_bytes: row.get(4)?,
        thread_count: row.get(5)?,
    })
}

fn map_error_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<ErrorEvent> {
    Ok(ErrorEvent {
        id: row.get(0)?,
        event_type: row.get(1)?,
        severity: row.get(2)?,
        object_type: row.get(3)?,
        object_id: row.get(4)?,
        run_id: row.get(5)?,
        message: row.get(6)?,
        details_json: row.get(7)?,
        occurred_at: row.get(8)?,
        journal_cursor: row.get(9)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 按照 `migrations/` 中的结构创建一个临时数据库，用于验证只读查询语句。
    fn create_fixture_db(path: &Path) {
        let conn = Connection::open(path).expect("创建数据库");
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE process_runs (
                 id INTEGER PRIMARY KEY, run_id TEXT NOT NULL UNIQUE, service_name TEXT NOT NULL,
                 pid INTEGER NOT NULL, proc_start_ticks INTEGER NOT NULL, started_at INTEGER NOT NULL,
                 ended_at INTEGER, exit_code INTEGER, exit_signal INTEGER, runtime_ms INTEGER,
                 result TEXT NOT NULL DEFAULT 'running');
             CREATE TABLE process_metrics (
                 id INTEGER PRIMARY KEY, run_id TEXT NOT NULL, sampled_at INTEGER NOT NULL,
                 cpu_percent REAL NOT NULL DEFAULT 0.0, memory_bytes INTEGER NOT NULL DEFAULT 0,
                 thread_count INTEGER NOT NULL DEFAULT 0, read_bytes INTEGER, write_bytes INTEGER);
             CREATE TABLE error_events (
                 id INTEGER PRIMARY KEY, event_type TEXT NOT NULL, severity TEXT NOT NULL,
                 object_type TEXT NOT NULL, object_id TEXT NOT NULL, run_id TEXT, message TEXT NOT NULL,
                 details_json TEXT, occurred_at INTEGER NOT NULL, journal_cursor TEXT);
             INSERT INTO process_runs (run_id, service_name, pid, proc_start_ticks, started_at, result)
                 VALUES ('run-1', 'robot-lidar.service', 100, 5000, 1700000000, 'running');
             INSERT INTO process_runs (run_id, service_name, pid, proc_start_ticks, started_at, ended_at, exit_code, runtime_ms, result)
                 VALUES ('run-0', 'robot-lidar.service', 90, 4000, 1699999000, 1699999500, 1, 500000, 'failed');
             INSERT INTO process_metrics (run_id, sampled_at, cpu_percent, memory_bytes, thread_count)
                 VALUES ('run-1', 1700000005, 12.5, 1048576, 4);
             INSERT INTO error_events (event_type, severity, object_type, object_id, run_id, message, occurred_at)
                 VALUES ('process_exited_abnormally', 'error', 'process', 'robot-lidar.service', 'run-0', 'exit 1', 1699999500);",
        )
        .expect("建表成功");
    }

    #[test]
    fn missing_database_returns_none() {
        assert!(
            ReadOnlyDb::open(Path::new("/nonexistent/state.db"))
                .expect("不报错")
                .is_none()
        );
    }

    #[test]
    fn reads_runs_metrics_and_events() {
        let dir = tempfile::tempdir().expect("临时目录可用");
        let path = dir.path().join("state.db");
        create_fixture_db(&path);

        let db = ReadOnlyDb::open(&path).expect("可打开").expect("存在");
        let running = db.running_runs().expect("查询成功");
        assert_eq!(running.len(), 1);
        assert_eq!(running[0].run_id, "run-1");

        let history = db
            .process_runs(Some("robot-lidar.service"), 10)
            .expect("查询成功");
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].run_id, "run-1");

        let metrics = db
            .metrics_for_service("robot-lidar.service", 10)
            .expect("查询成功");
        assert_eq!(metrics.len(), 1);
        assert!((metrics[0].cpu_percent - 12.5).abs() < f64::EPSILON);

        let events = db.error_events(None, None, 10).expect("查询成功");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "process_exited_abnormally");
        assert_eq!(events[0].run_id.as_deref(), Some("run-0"));

        let filtered = db
            .error_events(Some("service_start_failed"), None, 10)
            .expect("查询成功");
        assert!(filtered.is_empty());
    }
}
