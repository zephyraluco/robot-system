//! 文件系统布局（见架构文档 §5.3）。
//!
//! 程序与静态资源放在 `/opt/robot-system`；数据库、运行数据与日志按 Linux 约定分别
//! 放在 `/var/lib`、`/run`、`/var/log` 下。

use std::path::{Path, PathBuf};

/// robot-system 使用的各目录与文件路径。
///
/// 这里的路径集合只覆盖 `rsctl` 实际用到的部分；常驻服务在自身 crate 中定义它需要的
/// 路径（两个程序独立实现，见 §2.3）。
#[derive(Debug, Clone)]
pub struct Paths {
    /// 程序与静态资源根目录，默认 `/opt/robot-system`。
    pub opt_root: PathBuf,
    /// 持久数据目录，默认 `/var/lib/robot-system`。
    pub var_lib: PathBuf,
    /// 运行时数据目录（tmpfs），默认 `/run/robot-system`。
    pub run: PathBuf,
}

impl Default for Paths {
    fn default() -> Self {
        Self {
            opt_root: PathBuf::from("/opt/robot-system"),
            var_lib: PathBuf::from("/var/lib/robot-system"),
            run: PathBuf::from("/run/robot-system"),
        }
    }
}

impl Paths {
    /// 探测路径配置。
    ///
    /// 若设置了环境变量 `ROBOT_SYSTEM_PREFIX`，则所有路径都以其为前缀重定向，
    /// 便于本地验证与测试；否则使用标准的系统路径。
    #[must_use]
    pub fn detect() -> Self {
        match std::env::var_os("ROBOT_SYSTEM_PREFIX") {
            Some(prefix) => Self::with_prefix(prefix),
            None => Self::default(),
        }
    }

    /// 以给定前缀构造一组路径（用于测试与本地验证）。
    #[must_use]
    pub fn with_prefix(prefix: impl AsRef<Path>) -> Self {
        let base = prefix.as_ref();
        Self {
            opt_root: base.join("opt/robot-system"),
            var_lib: base.join("var/lib/robot-system"),
            run: base.join("run/robot-system"),
        }
    }

    /// 覆盖程序根目录（来自 CLI `--opt-root`）。
    pub fn set_opt_root(&mut self, dir: impl Into<PathBuf>) {
        self.opt_root = dir.into();
    }

    /// 主配置文件。
    #[must_use]
    pub fn config_file(&self) -> PathBuf {
        self.opt_root.join("config/robot-system.conf")
    }

    /// 业务覆盖配置目录。
    #[must_use]
    pub fn config_apps_dir(&self) -> PathBuf {
        self.opt_root.join("config/apps")
    }

    /// 软件包声明的服务清单目录。
    #[must_use]
    pub fn packages_dir(&self) -> PathBuf {
        self.opt_root.join("packages")
    }

    /// SQLite 状态数据库。
    #[must_use]
    pub fn state_db(&self) -> PathBuf {
        self.var_lib.join("state.db")
    }

    /// 旧版本 DEB 与配置备份目录。
    #[must_use]
    pub fn backups_dir(&self) -> PathBuf {
        self.var_lib.join("backups")
    }

    /// 部署任务文件目录。
    #[must_use]
    pub fn transactions_dir(&self) -> PathBuf {
        self.var_lib.join("transactions")
    }

    /// 受管变更的应用级文件锁（见 §5.7）。
    #[must_use]
    pub fn lock_file(&self) -> PathBuf {
        self.run.join("lock")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_prefix_builds_expected_layout() {
        let paths = Paths::with_prefix("/tmp/rs-root");
        assert_eq!(
            paths.state_db(),
            PathBuf::from("/tmp/rs-root/var/lib/robot-system/state.db")
        );
        assert_eq!(
            paths.lock_file(),
            PathBuf::from("/tmp/rs-root/run/robot-system/lock")
        );
        assert_eq!(
            paths.packages_dir(),
            PathBuf::from("/tmp/rs-root/opt/robot-system/packages")
        );
    }
}
