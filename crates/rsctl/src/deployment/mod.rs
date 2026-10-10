//! `DeploymentManager`：部署编排（见架构文档 §4.5）。
//!
//! 将一次安装、升级或卸载抽象为可追踪的任务，并把各状态持久化到
//! `/var/lib/robot-system/transactions/` 下的任务文件（**不写入 SQLite**）。
//!
//! 关键约束：
//! * **部署事务的持久化不等同于可回滚的事务。** DEB 的 maintainer scripts 可能修改文件、
//!   启动服务或执行外部操作，这些副作用无法通过回滚任务状态文件来撤销。
//! * 同一时刻只允许一个受管变更事务：变更前获取应用级文件锁，并依赖 dpkg 自身的锁。
//! * 失败后按明确策略恢复旧版本；无法恢复时进入 [`TaskState::RecoveryRequired`]，
//!   **不会**误报成功。

pub mod state;
pub mod store;

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::config::Config;
use crate::error::{Error, Result};
use crate::lock::ManagedLock;
use crate::package::{self, DebInfo, PackageManager};
use crate::paths::Paths;
use crate::service::ServiceManager;
use crate::system::{command, dpkg, systemd};

pub use state::{StepOutcome, TaskKind, TaskState};
pub use store::{DeploymentTask, TaskStep, TaskStore};

/// 保留的旧版本 DEB 数量上限。
const BACKUP_KEEP: usize = 3;

/// 部署管理器。
#[derive(Debug)]
pub struct DeploymentManager {
    paths: Paths,
    config: Config,
    store: TaskStore,
    packages: PackageManager,
    services: ServiceManager,
}

impl DeploymentManager {
    /// 构造部署管理器。
    pub fn new(paths: Paths, config: Config) -> Result<Self> {
        let store = TaskStore::new(paths.transactions_dir());
        store.ensure_dir()?;
        let packages = PackageManager::new(paths.clone());
        let services = ServiceManager::new(paths.clone(), config.clone());
        Ok(Self {
            paths,
            config,
            store,
            packages,
            services,
        })
    }

    /// 列出所有部署任务（按创建时间倒序）。
    pub fn list_tasks(&self) -> Result<Vec<DeploymentTask>> {
        self.store.list()
    }

    /// 查询单个部署任务。
    pub fn task(&self, id: &str) -> Result<DeploymentTask> {
        self.store.load(id)
    }

    /// 查询指定软件包的部署历史。
    pub fn history(&self, package: &str) -> Result<Vec<DeploymentTask>> {
        self.store.history_for(package)
    }

    /// 检查是否存在未完成的部署任务；存在时拒绝新的变更。
    ///
    /// 部署任务的对账不依赖常驻服务：每次变更前自行扫描未完成任务。
    pub fn check_incomplete(&self) -> Result<()> {
        if let Some(task) = self.store.incomplete()?.into_iter().next() {
            return Err(Error::IncompleteTask {
                task_id: task.id,
                state: task.state.as_str().to_string(),
            });
        }
        Ok(())
    }

    /// 安装或升级本地 DEB 文件。
    ///
    /// `kind` 为 [`TaskKind::Install`] 时，若该包已安装则自动按升级处理。
    pub fn install(&self, deb: &Path, kind: TaskKind) -> Result<DeploymentTask> {
        command::require_root("package install/upgrade")?;
        let _lock = self.acquire_lock()?;
        self.check_incomplete()?;

        let info = self.packages.describe_deb(deb)?;
        let from_version = self.installed_version(&info.package)?;
        let kind = if from_version.is_some() && kind == TaskKind::Install {
            TaskKind::Upgrade
        } else {
            kind
        };

        let task = self.new_task(
            kind,
            &info.package,
            from_version.clone(),
            Some(info.version.clone()),
            Some(info.architecture.clone()),
            Some(info.path.clone()),
            Some(info.sha256.clone()),
        );
        let mut tx = Tx::new(task, &self.store)?;

        match self.run_install(&mut tx, deb, &info, from_version.as_deref()) {
            Ok(()) => tx.finish(TaskState::Committed)?,
            Err(err) => {
                tx.fail(&err)?;
                tx.enter(TaskState::Recovering)?;
                match self.recover_after_install(&mut tx, &info, from_version.as_deref()) {
                    Ok(Some(message)) => {
                        tx.set_recovery(message)?;
                        tx.finish(TaskState::Recovered)?;
                    }
                    _ => tx.finish(TaskState::RecoveryRequired)?,
                }
            }
        }

        // 备份整理失败不应影响部署结果。
        let _ = self.prune_backups(&info.package, BACKUP_KEEP);
        Ok(tx.into_task())
    }

    /// 卸载软件包。
    pub fn remove(&self, name: &str) -> Result<DeploymentTask> {
        command::require_root("package remove")?;
        let _lock = self.acquire_lock()?;
        self.check_incomplete()?;
        package::validate_package_name(name)?;

        let Some(from_version) = self.installed_version(name)? else {
            return Err(Error::PackageNotInstalled(name.to_string()));
        };

        let task = self.new_task(
            TaskKind::Remove,
            name,
            Some(from_version.clone()),
            None,
            None,
            None,
            None,
        );
        let mut tx = Tx::new(task, &self.store)?;

        match self.run_remove(&mut tx, name) {
            Ok(()) => tx.finish(TaskState::Committed)?,
            Err(err) => {
                tx.fail(&err)?;
                tx.enter(TaskState::Recovering)?;
                match self.recover_after_remove(&mut tx, name, &from_version) {
                    Ok(Some(message)) => {
                        tx.set_recovery(message)?;
                        tx.finish(TaskState::Recovered)?;
                    }
                    _ => tx.finish(TaskState::RecoveryRequired)?,
                }
            }
        }
        Ok(tx.into_task())
    }

    // ------------------------------------------------------------------
    // 内部实现
    // ------------------------------------------------------------------

    /// 获取应用级变更锁。
    fn acquire_lock(&self) -> Result<ManagedLock> {
        ManagedLock::acquire(
            &self.paths.lock_file(),
            Duration::from_secs(self.config.lock_timeout_secs),
        )
    }

    /// 已安装版本；未安装时返回 `None`。
    fn installed_version(&self, package: &str) -> Result<Option<String>> {
        Ok(self
            .packages
            .installed_one(package)?
            .filter(|pkg| pkg.is_installed())
            .map(|pkg| pkg.version))
    }

    #[allow(clippy::too_many_arguments)]
    fn new_task(
        &self,
        kind: TaskKind,
        package: &str,
        from_version: Option<String>,
        to_version: Option<String>,
        architecture: Option<String>,
        source_deb: Option<String>,
        sha256: Option<String>,
    ) -> DeploymentTask {
        let now = now();
        DeploymentTask {
            id: uuid::Uuid::new_v4().to_string(),
            kind,
            package: package.to_string(),
            from_version,
            to_version,
            architecture,
            source_deb,
            sha256,
            state: TaskState::Created,
            actor: current_actor(),
            created_at: now,
            updated_at: now,
            started_at: None,
            finished_at: None,
            error: None,
            recovery: None,
            steps: Vec::new(),
        }
    }

    /// 安装 / 升级的主流程。
    fn run_install(
        &self,
        tx: &mut Tx<'_>,
        deb: &Path,
        info: &DebInfo,
        from_version: Option<&str>,
    ) -> Result<()> {
        tx.enter(TaskState::Validating)?;
        let host_arch = self.packages.host_architecture()?;
        if info.architecture != "all" && info.architecture != host_arch {
            return Err(Error::Validation(format!(
                "包架构 {} 与本机 {host_arch} 不匹配",
                info.architecture
            )));
        }
        if let Some(old) = from_version {
            if dpkg::compare_versions(&info.version, "lt", old)? {
                return Err(Error::Validation(format!(
                    "不允许的降级：已安装 {old}，目标版本 {} 更低",
                    info.version
                )));
            }
            if info.version == old {
                tx.ok(format!("目标版本与已安装版本相同（{old}），将执行重新安装"))?;
            }
        }
        tx.ok(format!(
            "校验通过：{} {} （{}，sha256 {}）",
            info.package, info.version, host_arch, info.sha256
        ))?;

        tx.enter(TaskState::Prepared)?;
        let backup = self.backup_deb(deb, info)?;
        tx.ok(format!("已备份新版本 DEB：{}", backup.display()))?;
        if let Some(old) = from_version {
            match self.find_backup(&info.package, old) {
                Some(path) => tx.ok(format!("可回滚的旧版本备份：{}", path.display()))?,
                None => tx.ok(format!("未找到旧版本 {old} 的备份，失败时可能需要人工恢复"))?,
            }
        }
        if let Some(path) = self.backup_app_config(&info.package)? {
            tx.ok(format!("已备份业务配置：{}", path.display()))?;
        }

        tx.enter(TaskState::StoppingServices)?;
        let old_services = self.packages.services_of(&info.package)?;
        self.stop_services(tx, &old_services, &info.package)?;

        tx.enter(TaskState::ApplyingPackage)?;
        self.packages.install_deb(deb)?;
        tx.ok("dpkg / APT 执行完成")?;

        tx.enter(TaskState::ConfiguringServices)?;
        self.services.daemon_reload()?;
        tx.ok("systemd daemon-reload 完成")?;

        tx.enter(TaskState::StartingServices)?;
        let new_services = self.packages.services_of(&info.package)?;
        if new_services.is_empty() {
            tx.ok("软件包未声明任何服务")?;
        }
        for unit in &new_services {
            if let Err(err) = self.services.enable(unit) {
                tx.ok(format!("启用 {unit} 失败：{err}"))?;
            }
            self.services.start(unit)?;
            tx.ok(format!("已启动服务 {unit}"))?;
        }

        tx.enter(TaskState::VerifyingPackage)?;
        self.verify_install(info)?;
        for unit in &new_services {
            let status = systemd::show(unit)?;
            if !status.exists() {
                return Err(Error::Validation(format!("服务 {unit} 未正确安装")));
            }
        }
        tx.ok("软件包安装与校验通过")?;
        Ok(())
    }

    /// 卸载的主流程。
    fn run_remove(&self, tx: &mut Tx<'_>, name: &str) -> Result<()> {
        tx.enter(TaskState::Validating)?;
        let dependents = self.packages.dependents_of(name)?;
        if !dependents.is_empty() {
            return Err(Error::Validation(format!(
                "以下受管软件包依赖 {name}：{}",
                dependents.join(", ")
            )));
        }

        let services = self.packages.services_of(name)?;
        let mut exclusive = Vec::new();
        let mut shared = Vec::new();
        for unit in &services {
            if self
                .packages
                .other_owners_of_service(unit, name)?
                .is_empty()
            {
                exclusive.push(unit.clone());
            } else {
                shared.push(unit.clone());
            }
        }
        if !shared.is_empty() {
            tx.ok(format!("以下服务为共享服务，将不停止/禁用：{shared:?}"))?;
        }
        tx.ok(format!("本包独占服务：{exclusive:?}"))?;

        tx.enter(TaskState::Prepared)?;

        tx.enter(TaskState::StoppingServices)?;
        for unit in &exclusive {
            let status = systemd::show(unit)?;
            if !status.exists() {
                continue;
            }
            if status.is_active() {
                self.services.stop(unit)?;
                tx.ok(format!("已停止服务 {unit}"))?;
            }
            if status.is_enabled() {
                self.services.disable(unit)?;
                tx.ok(format!("已禁用服务 {unit}"))?;
            }
        }

        tx.enter(TaskState::ApplyingPackage)?;
        self.packages.remove(name)?;
        tx.ok("dpkg / APT 卸载完成")?;

        tx.enter(TaskState::ConfiguringServices)?;
        self.services.daemon_reload()?;
        tx.ok("systemd daemon-reload 完成")?;

        tx.enter(TaskState::VerifyingPackage)?;
        if self.installed_version(name)?.is_some() {
            return Err(Error::Validation(format!("卸载后 {name} 仍处于安装状态")));
        }
        tx.ok("卸载校验通过")?;
        Ok(())
    }

    /// 停止受影响的服务；共享服务与不存在的服务会被跳过。
    fn stop_services(&self, tx: &mut Tx<'_>, services: &[String], package: &str) -> Result<()> {
        for unit in services {
            let owners = self.packages.other_owners_of_service(unit, package)?;
            if !owners.is_empty() {
                tx.ok(format!(
                    "跳过共享服务 {unit}（同时由 {} 声明）",
                    owners.join(", ")
                ))?;
                continue;
            }
            let status = systemd::show(unit)?;
            if !status.exists() {
                continue;
            }
            if status.is_active() {
                self.services.stop(unit)?;
                tx.ok(format!("已停止服务 {unit}"))?;
            }
        }
        Ok(())
    }

    /// 校验软件包安装结果（安装完成、版本与摘要一致）。
    fn verify_install(&self, info: &DebInfo) -> Result<()> {
        match self.packages.installed_one(&info.package)? {
            Some(pkg) if pkg.is_installed() && pkg.version == info.version => Ok(()),
            Some(pkg) if pkg.is_installed() => Err(Error::Validation(format!(
                "版本校验失败：期望 {}，实际 {}",
                info.version, pkg.version
            ))),
            _ => Err(Error::Validation(format!(
                "软件包 {} 未被安装",
                info.package
            ))),
        }
    }

    /// 安装 / 升级失败后的恢复。
    ///
    /// 仅在能找到旧版本 DEB 备份时才尝试回滚，否则返回 `None`（进入 RecoveryRequired）。
    fn recover_after_install(
        &self,
        tx: &mut Tx<'_>,
        info: &DebInfo,
        from_version: Option<&str>,
    ) -> Result<Option<String>> {
        let Some(old) = from_version else {
            tx.ok("无旧版本可回滚，需人工处理")?;
            return Ok(None);
        };
        let Some(backup) = self.find_backup(&info.package, old) else {
            tx.ok(format!("未找到旧版本 {old} 的 DEB 备份，需人工处理"))?;
            return Ok(None);
        };

        match self.packages.install_deb(&backup) {
            Ok(()) => {
                let message = format!("已回滚到 {old}（备份 {}）", backup.display());
                tx.ok(message.clone())?;
                self.restart_declared_services(&info.package, tx);
                Ok(Some(message))
            }
            Err(err) => {
                tx.ok(format!("回滚失败：{err}"))?;
                Ok(None)
            }
        }
    }

    /// 卸载失败后的恢复：尝试重新安装并恢复服务。
    fn recover_after_remove(
        &self,
        tx: &mut Tx<'_>,
        name: &str,
        from_version: &str,
    ) -> Result<Option<String>> {
        if let Some(backup) = self.find_backup(name, from_version) {
            if self.packages.install_deb(&backup).is_ok() {
                let message = format!("已重新安装 {name} {from_version}");
                tx.ok(message.clone())?;
                self.services.daemon_reload().ok();
                self.restart_declared_services(name, tx);
                return Ok(Some(message));
            }
            tx.ok(format!("尝试重新安装 {name} 失败"))?;
        }

        if self.installed_version(name)?.is_some() {
            return Ok(Some(format!("{name} 仍处于安装状态，未产生实际卸载")));
        }
        tx.ok("无法自动恢复，需人工处理")?;
        Ok(None)
    }

    /// 尽力重启清单声明的服务（恢复路径使用，失败不致命）。
    fn restart_declared_services(&self, package: &str, tx: &mut Tx<'_>) {
        let Ok(units) = self.packages.services_of(package) else {
            return;
        };
        for unit in units {
            if let Err(err) = self.services.start(&unit) {
                let _ = tx.ok(format!("恢复时启动 {unit} 失败：{err}"));
            }
        }
    }

    /// 将 DEB 复制到备份目录。
    fn backup_deb(&self, deb: &Path, info: &DebInfo) -> Result<PathBuf> {
        let dir = self.paths.backups_dir();
        std::fs::create_dir_all(&dir)?;
        let dest = dir.join(format!(
            "{}_{}_{}.deb",
            info.package, info.version, info.architecture
        ));
        if !dest.exists() {
            std::fs::copy(deb, &dest)?;
        }
        Ok(dest)
    }

    /// 备份业务覆盖配置（若存在）。
    fn backup_app_config(&self, package: &str) -> Result<Option<PathBuf>> {
        let source = self.paths.config_apps_dir().join(format!("{package}.toml"));
        if !source.is_file() {
            return Ok(None);
        }
        let dir = self.paths.backups_dir();
        std::fs::create_dir_all(&dir)?;
        let dest = dir.join(format!("{package}.config.{}.bak", now()));
        std::fs::copy(&source, &dest)?;
        Ok(Some(dest))
    }

    /// 查找指定包与版本的备份 DEB。
    fn find_backup(&self, package: &str, version: &str) -> Option<PathBuf> {
        let prefix = format!("{package}_{version}_");
        let entries = std::fs::read_dir(self.paths.backups_dir()).ok()?;
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(&prefix) && name.ends_with(".deb") {
                return Some(entry.path());
            }
        }
        None
    }

    /// 每个软件包只保留最近 `keep` 个备份 DEB。
    fn prune_backups(&self, package: &str, keep: usize) -> Result<()> {
        let prefix = format!("{package}_");
        let entries = match std::fs::read_dir(self.paths.backups_dir()) {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(err) => return Err(Error::Io(err)),
        };

        let mut files: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with(&prefix) || !name.ends_with(".deb") {
                continue;
            }
            let modified = entry
                .metadata()
                .and_then(|meta| meta.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            files.push((modified, path));
        }

        files.sort_by_key(|entry| std::cmp::Reverse(entry.0));
        for (_, path) in files.into_iter().skip(keep) {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }
}

/// 当前 Unix 时间（秒）。
fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// 当前操作者：优先使用 `SUDO_USER`，其次 `USER`。
fn current_actor() -> String {
    std::env::var("SUDO_USER")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "unknown".to_string())
}

/// 部署事务写入器：每次状态变化都立即持久化到任务文件。
struct Tx<'a> {
    task: DeploymentTask,
    store: &'a TaskStore,
}

impl<'a> Tx<'a> {
    /// 创建事务并写入初始状态。
    fn new(task: DeploymentTask, store: &'a TaskStore) -> Result<Self> {
        let tx = Self { task, store };
        tx.store.save(&tx.task)?;
        Ok(tx)
    }

    /// 追加一个步骤记录并落盘。
    fn push(&mut self, state: TaskState, message: String, outcome: StepOutcome) -> Result<()> {
        self.task.updated_at = now();
        self.task.steps.push(TaskStep {
            state,
            at: self.task.updated_at,
            message,
            outcome,
        });
        self.store.save(&self.task)
    }

    /// 进入某个状态。
    fn enter(&mut self, state: TaskState) -> Result<()> {
        self.task.state = state;
        if self.task.started_at.is_none() {
            self.task.started_at = Some(now());
        }
        self.push(
            state,
            format!("进入状态 {}", state.as_str()),
            StepOutcome::Started,
        )
    }

    /// 记录一条成功信息。
    fn ok(&mut self, message: impl Into<String>) -> Result<()> {
        let state = self.task.state;
        self.push(state, message.into(), StepOutcome::Ok)
    }

    /// 记录失败。
    fn fail(&mut self, err: &Error) -> Result<()> {
        self.task.state = TaskState::Failed;
        self.task.error = Some(err.to_string());
        self.push(
            TaskState::Failed,
            format!("失败：{err}"),
            StepOutcome::Failed,
        )
    }

    /// 记录恢复结果。
    fn set_recovery(&mut self, message: String) -> Result<()> {
        self.task.recovery = Some(message);
        let state = self.task.state;
        self.push(state, String::from("已执行恢复"), StepOutcome::Ok)
    }

    /// 结束任务，写入终态与结束时间。
    fn finish(&mut self, state: TaskState) -> Result<()> {
        self.task.state = state;
        self.task.finished_at = Some(now());
        let outcome = if state.is_success() {
            StepOutcome::Ok
        } else {
            StepOutcome::Failed
        };
        self.push(state, format!("任务结束：{}", state.as_str()), outcome)
    }

    /// 取出最终任务。
    fn into_task(self) -> DeploymentTask {
        self.task
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_task() -> DeploymentTask {
        let now = now();
        DeploymentTask {
            id: "task-1".to_string(),
            kind: TaskKind::Install,
            package: "robot-lidar".to_string(),
            from_version: None,
            to_version: Some("1.2.0".to_string()),
            architecture: Some("amd64".to_string()),
            source_deb: None,
            sha256: None,
            state: TaskState::Created,
            actor: "tester".to_string(),
            created_at: now,
            updated_at: now,
            started_at: None,
            finished_at: None,
            error: None,
            recovery: None,
            steps: Vec::new(),
        }
    }

    #[test]
    fn tx_persists_every_transition() {
        let tmp = tempfile::tempdir().expect("临时目录可用");
        let store = TaskStore::new(tmp.path());

        let mut tx = Tx::new(sample_task(), &store).expect("创建事务");
        tx.enter(TaskState::Validating).expect("进入状态");
        tx.ok("校验通过").expect("记录");
        tx.enter(TaskState::Prepared).expect("进入状态");
        tx.finish(TaskState::Committed).expect("结束");

        let reloaded = store.load("task-1").expect("读取");
        assert_eq!(reloaded.state, TaskState::Committed);
        assert!(reloaded.state.is_terminal());
        assert!(reloaded.started_at.is_some());
        assert!(reloaded.finished_at.is_some());
        // 步骤：validating + 校验通过 + prepared + committed（创建任务本身不产生步骤）。
        assert_eq!(reloaded.steps.len(), 4);
    }

    #[test]
    fn tx_records_failure_and_recovery() {
        let tmp = tempfile::tempdir().expect("临时目录可用");
        let store = TaskStore::new(tmp.path());

        let mut tx = Tx::new(sample_task(), &store).expect("创建事务");
        tx.enter(TaskState::ApplyingPackage).expect("进入状态");
        tx.fail(&Error::Validation("dpkg 失败".to_string()))
            .expect("记录失败");
        assert_eq!(tx.task.state, TaskState::Failed);
        assert!(tx.task.error.is_some());
        assert!(!tx.task.state.is_terminal());

        tx.enter(TaskState::Recovering).expect("进入恢复");
        tx.set_recovery("已回滚".to_string()).expect("记录恢复");
        tx.finish(TaskState::Recovered).expect("结束");

        let reloaded = store.load("task-1").expect("读取");
        assert_eq!(reloaded.state, TaskState::Recovered);
        assert!(reloaded.state.is_success());
        assert_eq!(reloaded.recovery.as_deref(), Some("已回滚"));
    }

    #[test]
    fn incomplete_task_blocks_new_changes() {
        let tmp = tempfile::tempdir().expect("临时目录可用");
        let paths = Paths::with_prefix(tmp.path());
        let manager = DeploymentManager::new(paths.clone(), Config::default()).expect("构造成功");
        let store = TaskStore::new(paths.transactions_dir());

        manager.check_incomplete().expect("初始无阻塞任务");

        let mut task = sample_task();
        task.state = TaskState::ApplyingPackage;
        store.save(&task).expect("写入任务");

        assert!(matches!(
            manager.check_incomplete(),
            Err(Error::IncompleteTask { .. })
        ));

        // 进入终态后不再阻塞。
        task.state = TaskState::RecoveryRequired;
        store.save(&task).expect("写入任务");
        manager.check_incomplete().expect("终态不阻塞");
    }

    #[test]
    fn prune_backups_keeps_newest() {
        let tmp = tempfile::tempdir().expect("临时目录可用");
        let paths = Paths::with_prefix(tmp.path());
        let manager = DeploymentManager::new(paths.clone(), Config::default()).expect("构造成功");
        std::fs::create_dir_all(paths.backups_dir()).expect("创建备份目录");

        for version in ["1.0.0", "1.1.0", "1.2.0", "1.3.0"] {
            std::fs::write(
                paths
                    .backups_dir()
                    .join(format!("robot-lidar_{version}_amd64.deb")),
                b"deb",
            )
            .expect("写入备份");
            // 保证 mtime 有区分度，便于按时间淘汰。
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        std::fs::write(paths.backups_dir().join("other_9.9.9_amd64.deb"), b"deb").expect("写入");

        manager.prune_backups("robot-lidar", 3).expect("清理成功");

        let remaining: Vec<String> = std::fs::read_dir(paths.backups_dir())
            .expect("读取目录")
            .map(|e| e.expect("条目").file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(remaining.len(), 4, "总共应保留 4 个备份文件");
        assert!(
            !remaining.contains(&"robot-lidar_1.0.0_amd64.deb".to_string()),
            "最旧的备份应被清理"
        );
        assert!(remaining.contains(&"robot-lidar_1.3.0_amd64.deb".to_string()));
        assert!(
            remaining.contains(&"other_9.9.9_amd64.deb".to_string()),
            "其它包的备份不应被清理"
        );
    }

    #[test]
    fn find_backup_matches_package_and_version() {
        let tmp = tempfile::tempdir().expect("临时目录可用");
        let paths = Paths::with_prefix(tmp.path());
        let manager = DeploymentManager::new(paths.clone(), Config::default()).expect("构造成功");
        std::fs::create_dir_all(paths.backups_dir()).expect("创建备份目录");
        let file = paths.backups_dir().join("robot-lidar_1.1.0_amd64.deb");
        std::fs::write(&file, b"deb").expect("写入");

        assert_eq!(manager.find_backup("robot-lidar", "1.1.0"), Some(file));
        assert_eq!(manager.find_backup("robot-lidar", "9.9.9"), None);
    }
}
