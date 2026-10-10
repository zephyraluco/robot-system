//! 受管变更的应用级串行化锁（见架构文档 §4.6）。
//!
//! `/run/robot-system/lock` 是**应用级锁**，只能约束**遵守该锁的 `rsctl` 进程**。
//! 它无法阻止管理员直接使用 APT / dpkg、`systemctl` 或 `unattended-upgrades`，
//! 也无法替代 dpkg 自身的锁。这一点在文档中已明确说明。
//!
//! 只读查询**不加**此锁，以避免查询与变更相互阻塞。

use std::fs::{File, OpenOptions};
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::error::{Error, Result};

/// 持有期间保持排他锁的文件句柄；`Drop` 时自动释放。
///
/// 该句柄仅用于维持内核锁，不参与其它读取，因此以 `_` 前缀命名。
#[derive(Debug)]
pub struct ManagedLock {
    _file: File,
}

impl ManagedLock {
    /// 获取应用级变更锁。
    ///
    /// * `path` 为锁文件路径（`/run/robot-system/lock`）。
    /// * `timeout` 为最长等待时间；超时后返回 [`Error::LockBusy`]。
    ///
    /// 该函数会创建锁文件所在目录。
    pub fn acquire(path: &Path, timeout: Duration) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;

        let deadline = Instant::now() + timeout;
        loop {
            if try_lock_exclusive(&file)? {
                return Ok(Self { _file: file });
            }
            if Instant::now() >= deadline {
                return Err(Error::LockBusy(format!(
                    "{}（等待 {}s 后仍未获取到锁；可能已有另一个 rsctl 变更正在进行）",
                    path.display(),
                    timeout.as_secs()
                )));
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }
}

/// 获取排他 `flock`，非阻塞；返回是否成功获取。
fn try_lock_exclusive(file: &File) -> Result<bool> {
    // SAFETY: `file` 持有有效的文件描述符，`flock` 只在该描述符上设置内核锁。
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc == 0 {
        return Ok(true);
    }
    let err = std::io::Error::last_os_error();
    match err.raw_os_error() {
        // 已被其它进程持有。
        Some(libc::EWOULDBLOCK) => Ok(false),
        _ => Err(Error::Io(err)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_parent_directory_on_demand() {
        // 锁位于 tmpfs 上的 /run/robot-system，重启即消失；因此获取锁时必须能自建目录，
        // 不依赖任何预先存在的运行时目录。
        let dir = tempfile::tempdir().expect("临时目录可用");
        let path = dir.path().join("nested/robot-system/lock");
        let parent = path.parent().expect("有父目录").to_path_buf();
        assert!(!parent.exists(), "前置条件：目录尚不存在");

        let lock = ManagedLock::acquire(&path, Duration::from_millis(0)).expect("应自动创建目录");
        assert!(parent.is_dir(), "父目录应被创建");
        assert!(path.is_file(), "锁文件应被创建");

        drop(lock);
    }

    #[test]
    fn second_acquire_times_out_while_first_held() {
        let dir = tempfile::tempdir().expect("临时目录可用");
        let path = dir.path().join("lock");

        let first = ManagedLock::acquire(&path, Duration::from_millis(0)).expect("首次获取成功");
        let second = ManagedLock::acquire(&path, Duration::from_millis(50));
        assert!(
            matches!(second, Err(Error::LockBusy(_))),
            "同一进程重复 flock 应失败"
        );

        drop(first);
    }
}
