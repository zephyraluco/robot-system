//! `/proc` 适配层（见架构文档 §4.2、§4.3）。
//!
//! 由于 Linux 会复用 PID，**不能只用 PID 标识进程**。本模块提供进程启动时钟值
//! （`/proc/<pid>/stat` 的 `starttime`）用于区分不同的进程实例。

use std::path::PathBuf;

use serde::Serialize;

use crate::error::Result;

/// `/proc/<pid>/stat` 中与本系统相关的字段。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProcStat {
    /// 进程状态字符（R/S/D/Z/T 等）。
    pub state: char,
    /// 父进程 PID。
    pub ppid: i64,
    /// 用户态 CPU 时间（时钟滴答）。
    pub utime: u64,
    /// 内核态 CPU 时间（时钟滴答）。
    pub stime: u64,
    /// 线程数。
    pub num_threads: u64,
    /// 内核进程启动时钟值（自开机起的时钟滴答）。
    pub start_ticks: u64,
    /// 常驻内存页数。
    pub rss_pages: i64,
}

/// 每秒钟的时钟滴答数（`_SC_CLK_TCK`）。
#[must_use]
pub fn clock_ticks() -> u64 {
    // SAFETY: `sysconf` 只读取系统常量，无副作用。
    let value = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if value > 0 { value as u64 } else { 100 }
}

/// 系统启动时间（Unix 秒），来自 `/proc/stat` 的 `btime`。
pub fn boot_time_secs() -> Option<i64> {
    let text = std::fs::read_to_string("/proc/stat").ok()?;
    text.lines()
        .find_map(|line| line.strip_prefix("btime "))
        .and_then(|value| value.trim().parse::<i64>().ok())
}

/// 读取指定进程的 `/proc/<pid>/stat`；进程不存在时返回 `None`。
pub fn read_stat(pid: u32) -> Result<Option<ProcStat>> {
    let path = PathBuf::from(format!("/proc/{pid}/stat"));
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => return Ok(None),
        Err(err) => return Err(err.into()),
    };
    Ok(parse_stat(&text))
}

/// 进程的启动时间（Unix 秒）。
pub fn start_time_unix(pid: u32) -> Option<i64> {
    let stat = read_stat(pid).ok().flatten()?;
    let boot = boot_time_secs()?;
    let ticks = clock_ticks();
    if ticks == 0 {
        return None;
    }
    Some(boot + (stat.start_ticks / ticks) as i64)
}

/// 从 `/proc/<pid>/stat` 的内容解析所需字段。
///
/// 注意：`comm` 字段可能包含空格与括号，因此先用首尾括号定位，再按空白切分其后的字段。
#[must_use]
pub fn parse_stat(text: &str) -> Option<ProcStat> {
    let open = text.find('(')?;
    let close = text.rfind(')')?;
    if close < open {
        return None;
    }
    let rest = text.get(close + 1..)?.trim();
    let fields: Vec<&str> = rest.split_whitespace().collect();

    // `rest` 的第一个字段对应内核 `stat` 的第 3 个字段（state）。
    let field = |index_from_state: usize| -> Option<&str> { fields.get(index_from_state).copied() };

    let state = field(0)?.chars().next()?;
    let ppid = field(1)?.parse().ok()?;
    let utime = field(11)?.parse().ok()?;
    let stime = field(12)?.parse().ok()?;
    let num_threads = field(17)?.parse().ok()?;
    let start_ticks = field(19)?.parse().ok()?;
    let rss_pages = field(21)?.parse().ok()?;

    Some(ProcStat {
        state,
        ppid,
        utime,
        stime,
        num_threads,
        start_ticks,
        rss_pages,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_stat_with_plain_comm() {
        let text = "1234 (bash) S 1 1234 1234 34816 1234 4194304 1000 0 0 0 10 20 0 0 20 0 1 0 5000 12345678 1000 18446744073709551615";
        let stat = parse_stat(text).expect("可解析");
        assert_eq!(stat.state, 'S');
        assert_eq!(stat.ppid, 1);
        assert_eq!(stat.utime, 10);
        assert_eq!(stat.stime, 20);
        assert_eq!(stat.num_threads, 1);
        assert_eq!(stat.start_ticks, 5000);
        assert_eq!(stat.rss_pages, 1000);
    }

    #[test]
    fn parses_stat_with_spaces_in_comm() {
        let text = "9 (weird (name) here) R 0 9 9 0 -1 4194304 0 0 0 0 1 2 0 0 20 0 3 0 777 0 5 0";
        let stat = parse_stat(text).expect("可解析");
        assert_eq!(stat.state, 'R');
        assert_eq!(stat.utime, 1);
        assert_eq!(stat.stime, 2);
        assert_eq!(stat.num_threads, 3);
        assert_eq!(stat.start_ticks, 777);
        assert_eq!(stat.rss_pages, 5);
    }

    #[test]
    fn rejects_truncated_input() {
        assert!(parse_stat("1234 (bash) S 1").is_none());
        assert!(parse_stat("garbage").is_none());
    }

    #[test]
    fn system_constants_are_sane() {
        assert!(clock_ticks() >= 1);
        assert!(boot_time_secs().is_some());
    }

    #[test]
    fn missing_pid_returns_none() {
        // PID 1 必然存在，构造一个几乎不可能存在的 PID。
        assert!(read_stat(u32::MAX).expect("读取不报错").is_none());
    }
}
