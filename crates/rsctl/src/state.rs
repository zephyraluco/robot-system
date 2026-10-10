//! 运行状态的**只读**视图（见架构文档 §5.1）。
//!
//! 汇总来自 SQLite 的进程运行、指标与错误事件历史。数据库缺失时（常驻服务从未运行）
//! 不会报错，而是标记为“历史不可用”，由调用方回退为直接查询 systemd / journald / `/proc`。

use std::path::{Path, PathBuf};

use crate::db::{ErrorEvent, MetricSample, ProcessRun, ReadOnlyDb};
use crate::error::Result;
use crate::paths::Paths;

/// 运行状态仓库（只读）。
#[derive(Debug)]
pub struct StateStore {
    db: Option<ReadOnlyDb>,
    path: PathBuf,
}

impl StateStore {
    /// 打开状态数据库；文件不存在时进入“历史不可用”状态。
    pub fn open(paths: &Paths) -> Result<Self> {
        let path = paths.state_db();
        Ok(Self {
            db: ReadOnlyDb::open(&path)?,
            path,
        })
    }

    /// 历史数据是否可用。
    #[must_use]
    pub fn available(&self) -> bool {
        self.db.is_some()
    }

    /// 数据库文件路径。
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 进程运行历史。
    pub fn process_runs(&self, service: Option<&str>, limit: u32) -> Result<Vec<ProcessRun>> {
        match &self.db {
            Some(db) => db.process_runs(service, limit),
            None => Ok(Vec::new()),
        }
    }

    /// 当前 `running` 状态的运行实例。
    pub fn running_runs(&self) -> Result<Vec<ProcessRun>> {
        match &self.db {
            Some(db) => db.running_runs(),
            None => Ok(Vec::new()),
        }
    }

    /// 指定服务的最近指标采样。
    pub fn metrics_for_service(&self, service: &str, limit: u32) -> Result<Vec<MetricSample>> {
        match &self.db {
            Some(db) => db.metrics_for_service(service, limit),
            None => Ok(Vec::new()),
        }
    }

    /// 错误事件。
    pub fn error_events(
        &self,
        event_type: Option<&str>,
        object: Option<&str>,
        limit: u32,
    ) -> Result<Vec<ErrorEvent>> {
        match &self.db {
            Some(db) => db.error_events(event_type, object, limit),
            None => Ok(Vec::new()),
        }
    }
}
