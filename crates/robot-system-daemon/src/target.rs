//! 受管 target 的服务发现（见架构文档 §2、§4.2、§4.3）。
//!
//! 业务服务的 unit 文件由各自的 DEB 提供，本系统**不保存业务服务的副本**；常驻服务
//! 因此以 systemd 的依赖关系为准，通过 `systemctl list-dependencies <target>` 的直接
//! 依赖确定需要监听哪些服务。

use crate::error::Result;
use crate::system::systemd;

/// 受管 target 下的服务集合。
#[derive(Debug, Clone)]
pub struct TargetServices {
    target: String,
}

impl TargetServices {
    /// 以 target 名称构造。
    #[must_use]
    pub fn new(target: impl Into<String>) -> Self {
        Self {
            target: target.into(),
        }
    }

    /// 当前需要监听的服务单元名（去重后按名称排序）。
    ///
    /// target 尚未安装时不返回错误，而是给出空列表——此时系统里确实还没有受管服务。
    pub fn units(&self) -> Result<Vec<String>> {
        systemd::service_dependencies(&self.target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_target_yields_no_units() {
        let services = TargetServices::new("robot-system-nonexistent-test.target");
        assert!(services.units().expect("查询成功").is_empty());
    }

    #[test]
    fn rejects_illegal_target_names() {
        let services = TargetServices::new("bad; rm -rf /");
        assert!(services.units().is_err());
    }
}
