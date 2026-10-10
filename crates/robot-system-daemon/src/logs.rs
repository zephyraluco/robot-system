//! `LogManager`：日志记录与异常事件整理（见架构文档 §4.4）。
//!
//! 原始日志由 journald 保存，本模块只做两件事：
//! * 按服务增量读取 journald 中**warning 及以上**级别的日志；
//! * 将关键异常整理为结构化错误事件写入 SQLite。
//!
//! 增量读取依据上一次记录到的 journal 游标（`--after-cursor`），避免重复导入。

use crate::db::Database;
use crate::error::Result;
use crate::event;
use crate::system::journald;

/// 一次日志扫描的结果。
#[derive(Debug, Default, Clone, Copy)]
pub struct LogScan {
    /// 本次读取到的日志条数。
    pub scanned: usize,
    /// 本次新记录的事件数。
    pub recorded: usize,
}

/// 日志管理器。
#[derive(Debug, Clone)]
pub struct LogManager {
    /// 回溯时间窗口（秒）。
    lookback_secs: i64,
    /// 单次读取的最大条数。
    max_lines: usize,
    /// 记录为事件的最低优先级门槛（syslog：4 = warning）。
    min_priority: u8,
}

impl LogManager {
    /// 构造日志管理器。
    #[must_use]
    pub fn new(lookback_secs: i64, max_lines: usize) -> Self {
        Self {
            lookback_secs,
            max_lines,
            min_priority: 4,
        }
    }

    /// 扫描单个服务的日志并整理错误事件。
    pub fn scan(&self, database: &Database, unit: &str) -> Result<LogScan> {
        let cursor = database.latest_cursor(unit)?;
        let since = crate::db::now() - self.lookback_secs.max(0);

        let entries = journald::query(
            unit,
            Some(since),
            cursor.as_deref(),
            Some("warning"),
            self.max_lines,
        )?;

        let mut scan = LogScan {
            scanned: entries.len(),
            recorded: 0,
        };
        for entry in &entries {
            // `-p warning` 已做过滤，这里再保证一次，避免不同 journalctl 行为差异。
            if entry.priority > self.min_priority {
                continue;
            }
            if database.has_recent_event(event::SERVICE_LOG_ERROR, unit, entry.timestamp)? {
                continue;
            }
            event::service_log_error(unit, entry).record(database)?;
            scan.recorded += 1;
        }
        Ok(scan)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manager_defaults_are_reasonable() {
        let manager = LogManager::new(300, 200);
        assert_eq!(manager.lookback_secs, 300);
        assert_eq!(manager.max_lines, 200);
        assert_eq!(manager.min_priority, 4);
    }
}
