//! 部署任务的状态机定义（见架构文档 §4.5、§4.2）。

use serde::{Deserialize, Serialize};

/// 部署任务类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    /// 安装。
    Install,
    /// 升级。
    Upgrade,
    /// 卸载。
    Remove,
}

impl TaskKind {
    /// 字符串形式。
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::Upgrade => "upgrade",
            Self::Remove => "remove",
        }
    }
}

/// 部署任务的执行状态。
///
/// 正常路径：
/// `created → validating → prepared → stopping_services → applying_package →
/// configuring_services → starting_services → verifying_package → committed`
///
/// 失败路径：`failed → recovering → recovered | recovery_required`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    /// 已创建。
    Created,
    /// 校验来源、摘要、架构、依赖。
    Validating,
    /// 已保存恢复信息。
    Prepared,
    /// 正在停止受影响服务。
    StoppingServices,
    /// 正在执行 dpkg / APT。
    ApplyingPackage,
    /// 正在刷新 systemd 配置。
    ConfiguringServices,
    /// 正在启动服务。
    StartingServices,
    /// 正在校验安装结果。
    VerifyingPackage,
    /// 已提交（成功）。
    Committed,
    /// 失败。
    Failed,
    /// 正在恢复。
    Recovering,
    /// 已恢复。
    Recovered,
    /// 需要人工恢复。
    RecoveryRequired,
}

impl TaskState {
    /// 字符串形式（与序列化保持一致）。
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Validating => "validating",
            Self::Prepared => "prepared",
            Self::StoppingServices => "stopping_services",
            Self::ApplyingPackage => "applying_package",
            Self::ConfiguringServices => "configuring_services",
            Self::StartingServices => "starting_services",
            Self::VerifyingPackage => "verifying_package",
            Self::Committed => "committed",
            Self::Failed => "failed",
            Self::Recovering => "recovering",
            Self::Recovered => "recovered",
            Self::RecoveryRequired => "recovery_required",
        }
    }

    /// 是否为终态。终态任务不再阻塞后续变更。
    #[must_use]
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Committed | Self::Recovered | Self::RecoveryRequired
        )
    }

    /// 是否以成功告终。
    #[must_use]
    pub fn is_success(self) -> bool {
        matches!(self, Self::Committed | Self::Recovered)
    }
}

/// 单个步骤的执行结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepOutcome {
    /// 进入该状态。
    Started,
    /// 正常完成。
    Ok,
    /// 失败。
    Failed,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_states_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&TaskState::StartingServices).unwrap(),
            "\"starting_services\""
        );
        assert_eq!(
            serde_json::to_string(&TaskState::RecoveryRequired).unwrap(),
            "\"recovery_required\""
        );
        assert_eq!(
            serde_json::to_string(&TaskKind::Upgrade).unwrap(),
            "\"upgrade\""
        );
    }

    #[test]
    fn terminal_and_success_classification() {
        assert!(TaskState::Committed.is_terminal());
        assert!(TaskState::Recovered.is_terminal());
        assert!(TaskState::RecoveryRequired.is_terminal());
        assert!(!TaskState::Failed.is_terminal());
        assert!(!TaskState::Validating.is_terminal());

        assert!(TaskState::Committed.is_success());
        assert!(TaskState::Recovered.is_success());
        assert!(!TaskState::RecoveryRequired.is_success());
    }

    #[test]
    fn as_str_matches_serde() {
        for state in [
            TaskState::Created,
            TaskState::Validating,
            TaskState::Prepared,
            TaskState::StoppingServices,
            TaskState::ApplyingPackage,
            TaskState::ConfiguringServices,
            TaskState::StartingServices,
            TaskState::VerifyingPackage,
            TaskState::Committed,
            TaskState::Failed,
            TaskState::Recovering,
            TaskState::Recovered,
            TaskState::RecoveryRequired,
        ] {
            let json = serde_json::to_string(&state).expect("可序列化");
            assert_eq!(json, format!("\"{}\"", state.as_str()));
        }
    }
}
