//! 常驻服务使用的文件系统布局（见架构文档 §5.3）。
//!
//! 只包含常驻服务实际需要的路径：数据库迁移、业务服务清单、状态数据库与自身日志。
//! `rsctl` 在其自身 crate 中定义它需要的路径（两个程序独立实现）。

use std::path::PathBuf;

/// 常驻服务的路径集合。
#[derive(Debug, Clone)]
pub struct Paths {
    /// 程序与静态资源根目录，默认 `/opt/robot-system`。
    pub opt_root: PathBuf,
    /// 持久数据目录，默认 `/var/lib/robot-system`。
    pub var_lib: PathBuf,
    /// 日志目录，默认 `/var/log/robot-system`。
    pub log: PathBuf,
}

impl Default for Paths {
    fn default() -> Self {
        Self {
            opt_root: PathBuf::from("/opt/robot-system"),
            var_lib: PathBuf::from("/var/lib/robot-system"),
            log: PathBuf::from("/var/log/robot-system"),
        }
    }
}

impl Paths {
    /// 探测路径配置。
    ///
    /// 若设置了环境变量 `ROBOT_SYSTEM_PREFIX`，则所有路径都以其为前缀重定向，便于本地
    /// 验证与测试；否则使用标准的系统路径。
    #[must_use]
    pub fn detect() -> Self {
        match std::env::var_os("ROBOT_SYSTEM_PREFIX") {
            Some(prefix) => Self::with_prefix(prefix),
            None => Self::default(),
        }
    }

    /// 以给定前缀构造一组路径（用于测试与本地验证）。
    #[must_use]
    pub fn with_prefix(prefix: impl AsRef<std::path::Path>) -> Self {
        let base = prefix.as_ref();
        Self {
            opt_root: base.join("opt/robot-system"),
            var_lib: base.join("var/lib/robot-system"),
            log: base.join("var/log/robot-system"),
        }
    }

    /// 主配置文件。
    #[must_use]
    pub fn config_file(&self) -> PathBuf {
        self.opt_root.join("config/robot-system.conf")
    }

    /// 数据库结构迁移脚本目录。
    #[must_use]
    pub fn migrations_dir(&self) -> PathBuf {
        self.opt_root.join("migrations")
    }

    /// SQLite 状态数据库。
    #[must_use]
    pub fn state_db(&self) -> PathBuf {
        self.var_lib.join("state.db")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_prefix_builds_expected_layout() {
        let paths = Paths::with_prefix("/tmp/rs");
        assert_eq!(
            paths.state_db(),
            PathBuf::from("/tmp/rs/var/lib/robot-system/state.db")
        );
        assert_eq!(
            paths.migrations_dir(),
            PathBuf::from("/tmp/rs/opt/robot-system/migrations")
        );
    }
}
