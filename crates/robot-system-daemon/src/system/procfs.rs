//! `/proc` 适配层（见架构文档 §4.2、§4.3）。
//!
//! 采集进程启动时钟值、CPU 时间、内存与线程数。**不能只用 PID 标识进程**，因为 Linux
//! 会复用 PID，因此运行实例的身份由 `run_id + pid + 启动时钟值` 共同确定。

use std::path::PathBuf;

use crate::error::Result;

/// `/proc/<pid>/stat` 中与本系统相关的字段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcStat {
    /// 进程状态字符。
    pub state: char,
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

impl ProcStat {
    /// 用户态 + 内核态 CPU 时间（时钟滴答）。
    #[must_use]
    pub fn cpu_ticks(&self) -> u64 {
        self.utime.saturating_add(self.stime)
    }

    /// 常驻内存（字节）。
    #[must_use]
    pub fn memory_bytes(&self) -> Option<u64> {
        if self.rss_pages < 0 {
            return None;
        }
        Some((self.rss_pages as u64).saturating_mul(page_size()))
    }
}

/// 每秒钟的时钟滴答数（`_SC_CLK_TCK`）。
#[must_use]
pub fn clock_ticks() -> u64 {
    // SAFETY: `sysconf` 只读取系统常量，无副作用。
    let value = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if value > 0 { value as u64 } else { 100 }
}

/// 系统内存页大小（字节）。
#[must_use]
pub fn page_size() -> u64 {
    // SAFETY: `sysconf` 只读取系统常量，无副作用。
    let value = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if value > 0 { value as u64 } else { 4096 }
}

/// 系统启动时间（Unix 秒），来自 `/proc/stat` 的 `btime`。
pub fn boot_time_secs() -> Option<i64> {
    let text = std::fs::read_to_string("/proc/stat").ok()?;
    text.lines()
        .find_map(|line| line.strip_prefix("btime "))
        .and_then(|value| value.trim().parse::<i64>().ok())
}

/// 读取指定进程的 `/proc/<pid>/stat`；进程已退出或不可读时返回 `None`。
pub fn read_stat(pid: u32) -> Result<Option<ProcStat>> {
    let path = PathBuf::from(format!("/proc/{pid}/stat"));
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err)
            if matches!(
                err.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
            ) =>
        {
            return Ok(None);
        }
        Err(err) => return Err(err.into()),
    };
    Ok(parse_stat(&text))
}

/// 进程启动时间（Unix 秒）。
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
/// `comm` 字段可能包含空格与括号，因此先用首尾括号定位，再按空白切分其后的字段。
#[must_use]
pub fn parse_stat(text: &str) -> Option<ProcStat> {
    let open = text.find('(')?;
    let close = text.rfind(')')?;
    if close < open {
        return None;
    }
    let fields: Vec<&str> = text.get(close + 1..)?.split_whitespace().collect();

    // `fields[0]` 对应内核 `stat` 的第 3 个字段（state）。
    let state = fields.first()?.chars().next()?;
    let utime = fields.get(11)?.parse().ok()?;
    let stime = fields.get(12)?.parse().ok()?;
    let num_threads = fields.get(17)?.parse().ok()?;
    let start_ticks = fields.get(19)?.parse().ok()?;
    let rss_pages = fields.get(21)?.parse().ok()?;

    Some(ProcStat {
        state,
        utime,
        stime,
        num_threads,
        start_ticks,
        rss_pages,
    })
}

/// 依据两次采样计算 CPU 使用率（百分比）。
///
/// `delta_ticks` 为两次采样之间消耗的 CPU 时钟滴答数，`elapsed_secs` 为真实经过时间。
#[must_use]
pub fn cpu_percent(delta_ticks: u64, elapsed_secs: f64, ticks_per_sec: u64) -> f64 {
    if elapsed_secs <= 0.0 || ticks_per_sec == 0 {
        return 0.0;
    }
    let cpu_secs = delta_ticks as f64 / ticks_per_sec as f64;
    (cpu_secs / elapsed_secs * 100.0).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_stat_with_spaces_in_comm() {
        let text =
            "9 (weird (name) here) R 0 9 9 0 -1 4194304 0 0 0 0 11 22 0 0 20 0 3 0 777 0 500 0";
        let stat = parse_stat(text).expect("可解析");
        assert_eq!(stat.state, 'R');
        assert_eq!(stat.utime, 11);
        assert_eq!(stat.stime, 22);
        assert_eq!(stat.cpu_ticks(), 33);
        assert_eq!(stat.num_threads, 3);
        assert_eq!(stat.start_ticks, 777);
        assert_eq!(stat.rss_pages, 500);
        assert_eq!(stat.memory_bytes(), Some(500 * page_size()));
    }

    #[test]
    fn rejects_truncated_input() {
        assert!(parse_stat("1234 (x) S 1").is_none());
        assert!(parse_stat("garbage").is_none());
    }

    #[test]
    fn cpu_percent_is_bounded_and_zero_on_first_sample() {
        // 1 秒内消耗 50 个滴答、每秒 100 滴答 → 50%
        assert!((cpu_percent(50, 1.0, 100) - 50.0).abs() < f64::EPSILON);
        assert!((cpu_percent(100, 1.0, 100) - 100.0).abs() < f64::EPSILON);
        assert!((cpu_percent(200, 1.0, 100) - 200.0).abs() < f64::EPSILON);
        assert!((cpu_percent(50, 0.0, 100)).abs() < f64::EPSILON);
    }

    #[test]
    fn negative_rss_yields_no_memory() {
        let text = "9 (x) R 0 9 9 0 -1 0 0 0 0 0 0 0 0 0 0 0 1 0 1 0 -5 0";
        let stat = parse_stat(text).expect("可解析");
        assert_eq!(stat.memory_bytes(), None);
    }

    #[test]
    fn system_constants_are_sane() {
        assert!(clock_ticks() >= 1);
        assert!(page_size() >= 1024);
        assert!(boot_time_secs().is_some());
    }

    #[test]
    fn missing_pid_returns_none() {
        assert!(read_stat(u32::MAX).expect("不报错").is_none());
    }
}
