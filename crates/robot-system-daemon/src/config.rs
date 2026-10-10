//! 常驻服务配置。
//!
//! 与 `rsctl` 共用同一个配置文件 `/etc/robot-system/robot-system.conf`，
//! 但各自只读取自己关心的字段（未声明的字段会被忽略），见架构文档 §5.3。

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// 常驻服务运行配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// 业务服务统一组织的 target（见 §4）。受管服务由该 target 的依赖关系确定。
    pub managed_target: String,
    /// 进程与资源采集周期（秒）。默认 5 秒（见 §5.4）。
    pub sample_interval_secs: u64,
    /// 每次日志巡检回溯的时间窗口（秒）。默认 300 秒。
    pub journal_lookback_secs: i64,
    /// 资源指标保留天数（见 §6.5）。
    pub metrics_retention_days: u64,
    /// 运行实例历史保留天数。
    pub runs_retention_days: u64,
    /// 运行类错误事件保留天数。
    pub events_retention_days: u64,
    /// 是否启用 HTTP 请求任务入口（当前仅占位，见 §10）。
    pub http_enabled: bool,
    /// HTTP 入口监听地址（仅当 `http_enabled` 为真时生效）。
    pub http_listen: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            managed_target: "robot-system.target".to_string(),
            sample_interval_secs: 5,
            journal_lookback_secs: 300,
            metrics_retention_days: 30,
            runs_retention_days: 90,
            events_retention_days: 180,
            http_enabled: false,
            http_listen: "127.0.0.1:8790".to_string(),
        }
    }
}

impl Config {
    /// 从路径加载配置；文件不存在时使用默认值。
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

    /// 采集周期。
    #[must_use]
    pub fn sample_interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.sample_interval_secs)
    }

    /// 校验配置取值。
    fn validate(&self) -> Result<()> {
        if self.sample_interval_secs == 0 {
            return Err(Error::Config("sample_interval_secs 必须大于 0".to_string()));
        }
        if self.http_enabled && self.http_listen.trim().is_empty() {
            return Err(Error::Config(
                "http_enabled 为真时 http_listen 不能为空".to_string(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_yields_defaults() {
        let config = Config::load(Path::new("/nonexistent/robot-system.conf")).expect("默认配置");
        assert_eq!(config.sample_interval_secs, 5);
        assert!(!config.http_enabled);
    }

    #[test]
    fn ignores_fields_belonging_to_rsctl() {
        // rsctl 会写入 lock_timeout_secs，常驻服务必须忽略它。
        let text = "lock_timeout_secs = 30\nsample_interval_secs = 2\n";
        let config: Config = toml::from_str(text).expect("可解析");
        assert_eq!(config.sample_interval_secs, 2);
        assert_eq!(config.managed_target, "robot-system.target");
    }

    #[test]
    fn rejects_zero_sample_interval() {
        let text = "sample_interval_secs = 0\n";
        let config: Config = toml::from_str(text).expect("可解析");
        assert!(matches!(config.validate(), Err(Error::Config(_))));
    }
}
