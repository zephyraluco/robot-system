//! `rsctl` 的配置读取。
//!
//! 配置文件为 TOML 格式，位于 `/etc/robot-system/robot-system.conf`。
//! 文件不存在时使用默认值，不视为错误。

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// `rsctl` 运行时配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// 应用级变更锁的等待超时（秒）。超时后报错而不是无限等待。
    pub lock_timeout_secs: u64,
    /// 业务服务统一组织的 target。
    pub managed_target: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            lock_timeout_secs: 60,
            managed_target: "robot-system.target".to_string(),
        }
    }
}

impl Config {
    /// 从给定路径加载配置；文件不存在时返回默认配置。
    pub fn load(path: &Path) -> Result<Self> {
        let config = match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).map_err(|source| Error::Toml {
                path: path.to_path_buf(),
                source,
            })?,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(err) => return Err(Error::Io(err)),
        };
        config.validate()?;
        Ok(config)
    }

    /// 校验配置取值。
    fn validate(&self) -> Result<()> {
        if self.lock_timeout_secs == 0 {
            return Err(Error::Config(
                "lock_timeout_secs 必须大于 0（否则变更锁会立即超时）".to_string(),
            ));
        }
        if self.managed_target.trim().is_empty() {
            return Err(Error::Config("managed_target 不能为空".to_string()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_yields_defaults() {
        let config =
            Config::load(Path::new("/nonexistent/robot-system.conf")).expect("默认配置可用");
        assert_eq!(config.managed_target, "robot-system.target");
        assert_eq!(config.lock_timeout_secs, 60);
    }

    #[test]
    fn parses_partial_document() {
        let text = r#"
            lock_timeout_secs = 5
        "#;
        let config: Config = toml::from_str(text).expect("部分字段可用");
        assert_eq!(config.lock_timeout_secs, 5);
        assert_eq!(config.managed_target, "robot-system.target");
    }

    #[test]
    fn rejects_zero_lock_timeout() {
        let dir = tempfile::tempdir().expect("临时目录可用");
        let path = dir.path().join("robot-system.conf");
        std::fs::write(&path, "lock_timeout_secs = 0\n").expect("写入");
        assert!(matches!(Config::load(&path), Err(Error::Config(_))));
    }
}
