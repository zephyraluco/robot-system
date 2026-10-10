//! 部署任务的文件持久化（见架构文档 §4.5、§5.2）。
//!
//! 部署任务与步骤事件**不写入 SQLite**，而是由 `rsctl` 以文件形式保存在
//! `/var/lib/robot-system/transactions/` 下，每个任务一个 JSON 文件。
//!
//! 写入采用“临时文件 + 原子 rename”，保证任务状态不会出现半写状态。

use std::io::Write;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::deployment::state::{StepOutcome, TaskKind, TaskState};
use crate::error::{Error, Result};

/// 部署任务中的一个步骤记录。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskStep {
    /// 该步骤对应的状态。
    pub state: TaskState,
    /// 发生时间（Unix 秒）。
    pub at: i64,
    /// 说明信息。
    pub message: String,
    /// 执行结果。
    pub outcome: StepOutcome,
}

/// 一次部署任务。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentTask {
    /// 任务 ID。
    pub id: String,
    /// 任务类型。
    pub kind: TaskKind,
    /// 目标软件包。
    pub package: String,
    /// 变更前的版本（升级 / 卸载时）。
    pub from_version: Option<String>,
    /// 变更后的目标版本（安装 / 升级时）。
    pub to_version: Option<String>,
    /// 包架构。
    pub architecture: Option<String>,
    /// 来源 DEB 文件路径。
    pub source_deb: Option<String>,
    /// 来源 DEB 的 SHA-256 摘要。
    pub sha256: Option<String>,
    /// 当前状态。
    pub state: TaskState,
    /// 操作者（`SUDO_USER` 或 `USER`）。
    pub actor: String,
    /// 创建时间。
    pub created_at: i64,
    /// 最近更新时间。
    pub updated_at: i64,
    /// 开始时间。
    pub started_at: Option<i64>,
    /// 结束时间。
    pub finished_at: Option<i64>,
    /// 失败原因。
    pub error: Option<String>,
    /// 恢复结果说明。
    pub recovery: Option<String>,
    /// 步骤记录。
    pub steps: Vec<TaskStep>,
}

impl DeploymentTask {
    /// 任务是否尚未进入终态。
    #[must_use]
    pub fn is_incomplete(&self) -> bool {
        !self.state.is_terminal()
    }
}

/// 部署任务文件仓库。
#[derive(Debug, Clone)]
pub struct TaskStore {
    dir: PathBuf,
}

impl TaskStore {
    /// 基于任务目录构造仓库。
    #[must_use]
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// 确保任务目录存在。
    pub fn ensure_dir(&self) -> Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        Ok(())
    }

    /// 任务 ID 对应的文件路径。
    #[must_use]
    pub fn path_for(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    /// 原子地写入任务文件。
    pub fn save(&self, task: &DeploymentTask) -> Result<()> {
        self.ensure_dir()?;
        let final_path = self.path_for(&task.id);
        let tmp_path = final_path.with_extension("json.tmp");
        let data = serde_json::to_vec_pretty(task)?;
        {
            let mut file = std::fs::File::create(&tmp_path)?;
            file.write_all(&data)?;
            file.sync_all()?;
        }
        std::fs::rename(&tmp_path, &final_path)?;
        Ok(())
    }

    /// 读取指定任务。
    pub fn load(&self, id: &str) -> Result<DeploymentTask> {
        let path = self.path_for(id);
        let text = std::fs::read_to_string(&path).map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                Error::InvalidArgument(format!("部署任务不存在：{id}"))
            } else {
                Error::Io(err)
            }
        })?;
        Ok(serde_json::from_str(&text)?)
    }

    /// 列出所有任务，按创建时间倒序。
    ///
    /// 无法解析的任务文件会被跳过，以保证列表可用。
    pub fn list(&self) -> Result<Vec<DeploymentTask>> {
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(Error::Io(err)),
        };

        let mut tasks = Vec::new();
        for entry in entries {
            let path = entry?.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            if let Ok(text) = std::fs::read_to_string(&path)
                && let Ok(task) = serde_json::from_str::<DeploymentTask>(&text)
            {
                tasks.push(task);
            }
        }
        tasks.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| b.id.cmp(&a.id))
        });
        Ok(tasks)
    }

    /// 列出尚未进入终态的任务。
    pub fn incomplete(&self) -> Result<Vec<DeploymentTask>> {
        Ok(self
            .list()?
            .into_iter()
            .filter(DeploymentTask::is_incomplete)
            .collect())
    }

    /// 列出指定软件包的部署历史。
    pub fn history_for(&self, package: &str) -> Result<Vec<DeploymentTask>> {
        Ok(self
            .list()?
            .into_iter()
            .filter(|task| task.package == package)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_task(id: &str, package: &str, state: TaskState, created_at: i64) -> DeploymentTask {
        DeploymentTask {
            id: id.to_string(),
            kind: TaskKind::Install,
            package: package.to_string(),
            from_version: None,
            to_version: Some("1.0.0".to_string()),
            architecture: Some("amd64".to_string()),
            source_deb: None,
            sha256: None,
            state,
            actor: "tester".to_string(),
            created_at,
            updated_at: created_at,
            started_at: Some(created_at),
            finished_at: None,
            error: None,
            recovery: None,
            steps: vec![TaskStep {
                state,
                at: created_at,
                message: "test".to_string(),
                outcome: StepOutcome::Started,
            }],
        }
    }

    #[test]
    fn round_trips_a_task() {
        let tmp = tempfile::tempdir().expect("临时目录可用");
        let store = TaskStore::new(tmp.path());
        let task = sample_task("t1", "robot-lidar", TaskState::Validating, 100);

        store.save(&task).expect("保存成功");
        let loaded = store.load("t1").expect("读取成功");
        assert_eq!(loaded.package, "robot-lidar");
        assert_eq!(loaded.state, TaskState::Validating);
        assert_eq!(loaded.steps.len(), 1);
    }

    #[test]
    fn lists_newest_first_and_filters_incomplete() {
        let tmp = tempfile::tempdir().expect("临时目录可用");
        let store = TaskStore::new(tmp.path());
        store
            .save(&sample_task("t1", "robot-a", TaskState::Committed, 100))
            .expect("保存");
        store
            .save(&sample_task(
                "t2",
                "robot-a",
                TaskState::ApplyingPackage,
                200,
            ))
            .expect("保存");
        store
            .save(&sample_task("t3", "robot-b", TaskState::Recovered, 300))
            .expect("保存");

        let all = store.list().expect("列表成功");
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].id, "t3");

        let incomplete = store.incomplete().expect("查询成功");
        assert_eq!(incomplete.len(), 1);
        assert_eq!(incomplete[0].id, "t2");

        let history = store.history_for("robot-a").expect("查询成功");
        assert_eq!(history.len(), 2);
    }

    #[test]
    fn leaves_no_temp_files_behind() {
        let tmp = tempfile::tempdir().expect("临时目录可用");
        let store = TaskStore::new(tmp.path());
        store
            .save(&sample_task("t1", "robot-a", TaskState::Committed, 1))
            .expect("保存");
        let names: Vec<String> = std::fs::read_dir(tmp.path())
            .expect("读取目录")
            .map(|e| e.expect("条目").file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["t1.json".to_string()]);
    }

    #[test]
    fn missing_task_reports_invalid_argument() {
        let tmp = tempfile::tempdir().expect("临时目录可用");
        let store = TaskStore::new(tmp.path());
        assert!(matches!(store.load("nope"), Err(Error::InvalidArgument(_))));
    }
}
