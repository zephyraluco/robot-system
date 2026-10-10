//! `EventManager`：运行类错误事件的统一记录（见架构文档 §4.4）。
//!
//! 只记录**运行类**事件（进程、服务）。软件包与部署类事件由 `rsctl` 写入部署任务文件，
//! 不进入本表。

use serde_json::json;

use crate::db::{self, Database};
use crate::error::Result;
use crate::system::{journald, systemd};

/// 事件类型：服务启动失败。
pub const SERVICE_START_FAILED: &str = "service_start_failed";
/// 事件类型：服务意外停止（进程正常退出但服务本应持续运行）。
pub const SERVICE_STOPPED_UNEXPECTEDLY: &str = "service_stopped_unexpectedly";
/// 事件类型：进程异常退出。
pub const PROCESS_EXITED_ABNORMALLY: &str = "process_exited_abnormally";
/// 事件类型：服务日志中出现错误级别记录（在进程/服务事件之外的扩展类型）。
pub const SERVICE_LOG_ERROR: &str = "service_log_error";

/// 事件严重级别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// 警告。
    Warning,
    /// 错误。
    Error,
    /// 严重。
    Critical,
}

impl Severity {
    /// 字符串形式。
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Warning => "warning",
            Self::Error => "error",
            Self::Critical => "critical",
        }
    }

    /// 由 syslog 优先级推导严重级别。
    #[must_use]
    pub fn from_priority(priority: u8) -> Self {
        match priority {
            0..=2 => Self::Critical,
            3 => Self::Error,
            _ => Self::Warning,
        }
    }
}

/// 一条待写入的错误事件。
#[derive(Debug, Clone)]
pub struct Event {
    /// 事件类型。
    pub event_type: String,
    /// 严重级别。
    pub severity: Severity,
    /// 对象类型：`service` 或 `process`。
    pub object_type: &'static str,
    /// 对象标识（服务名）。
    pub object_id: String,
    /// 关联的运行实例。
    pub run_id: Option<String>,
    /// 错误摘要。
    pub message: String,
    /// 结构化详情。
    pub details: serde_json::Value,
    /// 关联的 journal 游标。
    pub journal_cursor: Option<String>,
}

impl Event {
    /// 构造事件。
    #[must_use]
    pub fn new(
        event_type: &str,
        severity: Severity,
        object_type: &'static str,
        object_id: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            event_type: event_type.to_string(),
            severity,
            object_type,
            object_id: object_id.into(),
            run_id: None,
            message: message.into(),
            details: json!({}),
            journal_cursor: None,
        }
    }

    /// 关联运行实例。
    #[must_use]
    pub fn with_run(mut self, run_id: impl Into<String>) -> Self {
        self.run_id = Some(run_id.into());
        self
    }

    /// 附加结构化详情。
    #[must_use]
    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = details;
        self
    }

    /// 关联 journal 游标。
    #[must_use]
    pub fn with_cursor(mut self, cursor: impl Into<String>) -> Self {
        self.journal_cursor = Some(cursor.into());
        self
    }

    /// 写入数据库。
    pub fn record(&self, database: &Database) -> Result<()> {
        let details = serde_json::to_string(&self.details)?;
        database.insert_event(
            &self.event_type,
            self.severity.as_str(),
            self.object_type,
            &self.object_id,
            self.run_id.as_deref(),
            &self.message,
            Some(&details),
            db::now(),
        )
    }
}

/// 由进程退出信息构造“异常退出”事件。
#[must_use]
pub fn process_exited_abnormally(
    unit: &str,
    run_id: &str,
    exit_code: Option<i64>,
    exit_signal: Option<i64>,
) -> Event {
    let message = match (exit_code, exit_signal) {
        (Some(code), _) => format!("进程以退出码 {code} 结束"),
        (_, Some(signal)) => format!("进程被信号 {signal} 终止"),
        _ => "进程异常结束".to_string(),
    };
    Event::new(
        PROCESS_EXITED_ABNORMALLY,
        Severity::Error,
        "process",
        unit,
        message,
    )
    .with_run(run_id)
    .with_details(json!({ "exit_code": exit_code, "exit_signal": exit_signal }))
}

/// 由服务状态构造“服务意外停止”事件。
#[must_use]
pub fn service_stopped_unexpectedly(unit: &str, run_id: &str) -> Event {
    Event::new(
        SERVICE_STOPPED_UNEXPECTEDLY,
        Severity::Warning,
        "service",
        unit,
        "服务在未发生部署变更的情况下停止运行",
    )
    .with_run(run_id)
}

/// 由服务状态构造“服务启动失败”事件。
#[must_use]
pub fn service_start_failed(unit: &str, status: &systemd::ServiceStatus) -> Event {
    Event::new(
        SERVICE_START_FAILED,
        Severity::Error,
        "service",
        unit,
        "服务进入失败状态且未观察到运行实例",
    )
    .with_details(json!({
        "sub_state": status.sub_state,
        "result": status.result,
        "exec_main_status": status.exec_main_status,
    }))
}

/// 由日志条目构造“服务日志错误”事件。
#[must_use]
pub fn service_log_error(unit: &str, entry: &journald::LogEntry) -> Event {
    let mut event = Event::new(
        SERVICE_LOG_ERROR,
        Severity::from_priority(entry.priority),
        "service",
        unit,
        entry.message.clone(),
    )
    .with_details(json!({
        "priority": entry.priority,
        "pid": entry.pid,
        "identifier": entry.identifier,
    }));
    if let Some(cursor) = &entry.cursor {
        event = event.with_cursor(cursor.clone());
    }
    event
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
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
    fn severity_maps_from_syslog_priority() {
        assert_eq!(Severity::from_priority(0), Severity::Critical);
        assert_eq!(Severity::from_priority(3), Severity::Error);
        assert_eq!(Severity::from_priority(4), Severity::Warning);
        assert_eq!(Severity::from_priority(6), Severity::Warning);
    }

    #[test]
    fn records_event_with_details_and_cursor() {
        let (_tmp, db) = open_db();
        let event = process_exited_abnormally("robot-lidar.service", "run-1", Some(1), None);
        event.record(&db).expect("写入成功");

        let stored: (String, String, String, String) = db
            .raw_conn()
            .query_row(
                "SELECT event_type, severity, object_type, details_json FROM error_events",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("查询成功");
        assert_eq!(stored.0, PROCESS_EXITED_ABNORMALLY);
        assert_eq!(stored.1, "error");
        assert_eq!(stored.2, "process");
        assert!(stored.3.contains("\"exit_code\":1"));
    }

    #[test]
    fn start_failure_message_mentions_no_run_instance() {
        let status = systemd::ServiceStatus {
            sub_state: "failed".to_string(),
            result: "exit-code".to_string(),
            exec_main_status: 203,
            ..Default::default()
        };
        let event = service_start_failed("robot-lidar.service", &status);
        assert_eq!(event.event_type, SERVICE_START_FAILED);
        assert_eq!(event.details["exec_main_status"], 203);
    }

    #[test]
    fn log_error_keeps_cursor() {
        let entry = journald::LogEntry {
            timestamp: 100,
            priority: 3,
            message: "boom".to_string(),
            pid: Some(9),
            identifier: Some("robot-lidar".to_string()),
            cursor: Some("s=abc".to_string()),
        };
        let event = service_log_error("robot-lidar.service", &entry);
        assert_eq!(event.severity, Severity::Error);
        assert_eq!(event.journal_cursor.as_deref(), Some("s=abc"));
    }
}
