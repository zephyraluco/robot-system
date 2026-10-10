//! `ServiceManager`：systemd 服务生命周期管理（见架构文档 §3.1、§4.1）。
//!
//! 该模块只负责调用 systemd 并汇总状态，不直接操作数据库；服务状态以 systemd 为准。

use std::collections::BTreeMap;

use serde::Serialize;

use crate::config::Config;
use crate::error::{Error, Result};
use crate::package::PackageManifest;
use crate::paths::Paths;
use crate::system::systemd::{self, ServiceStatus, UnitEntry};

/// 受管服务的综合视图（清单归属 + systemd 状态）。
#[derive(Debug, Clone, Serialize)]
pub struct ManagedService {
    /// systemd 单元名。
    pub unit: String,
    /// 声明该服务的软件包列表（可能为多个，表示共享服务）。
    pub owners: Vec<String>,
    /// 加载状态。
    pub load_state: String,
    /// 活动状态。
    pub active_state: String,
    /// 子状态。
    pub sub_state: String,
    /// 启用状态。
    pub unit_file_state: String,
    /// 主进程 PID。
    pub main_pid: u32,
    /// 服务结果。
    pub result: String,
    /// 重启次数。
    pub n_restarts: u32,
}

impl ManagedService {
    /// 是否已启用。
    #[must_use]
    pub fn enabled(&self) -> bool {
        matches!(
            self.unit_file_state.as_str(),
            "enabled" | "enabled-runtime" | "static"
        )
    }
}

/// 服务管理器。
#[derive(Debug, Clone)]
pub struct ServiceManager {
    paths: Paths,
    config: Config,
}

impl ServiceManager {
    /// 构造服务管理器。
    #[must_use]
    pub fn new(paths: Paths, config: Config) -> Self {
        Self { paths, config }
    }

    /// 受管 target 名称。
    #[must_use]
    pub fn managed_target(&self) -> &str {
        &self.config.managed_target
    }

    /// 查询 target 状态。
    pub fn target_status(&self) -> Result<ServiceStatus> {
        systemd::show(self.managed_target())
    }

    /// 查询单个服务状态；服务不存在时返回 [`Error::ServiceNotFound`]。
    pub fn status(&self, unit: &str) -> Result<ServiceStatus> {
        systemd::validate_unit_name(unit)?;
        let status = systemd::show(unit)?;
        if !status.exists() {
            return Err(Error::ServiceNotFound(unit.to_string()));
        }
        Ok(status)
    }

    /// 当前纳入受管 target 的服务单元名（去重后按名称排序）。
    ///
    /// 来源是 systemd 的依赖关系（`systemctl list-dependencies <target>` 的直接依赖），
    /// 而不是本系统保存的副本：业务服务的 unit 文件由各自的 DEB 提供并通过
    /// `WantedBy=<target>` 归属到 target。
    pub fn target_units(&self) -> Result<Vec<String>> {
        systemd::service_dependencies(self.managed_target())
    }

    /// 列出受管 target 下的所有服务及其 systemd 状态。
    ///
    /// 服务集合来自 systemd 的依赖关系；软件包归属来自 DEB 提供的服务清单，仅用于
    /// 展示与归属判断（清单缺失时归属显示为空，不影响服务本身被管理）。
    pub fn list_managed(&self) -> Result<Vec<ManagedService>> {
        let mut owners: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for manifest in PackageManifest::load_all(&self.paths.packages_dir())? {
            for decl in manifest.services {
                owners
                    .entry(decl.name)
                    .or_default()
                    .push(manifest.package.clone());
            }
        }

        let units = self.target_units()?;
        let mut services = Vec::with_capacity(units.len());
        for unit in units {
            let status = systemd::show(&unit)?;
            services.push(ManagedService {
                unit: status.unit,
                owners: owners.remove(&unit).unwrap_or_default(),
                load_state: status.load_state,
                active_state: status.active_state,
                sub_state: status.sub_state,
                unit_file_state: status.unit_file_state,
                main_pid: status.main_pid,
                result: status.result,
                n_restarts: status.n_restarts,
            });
        }
        Ok(services)
    }

    /// 列出系统中所有 systemd service 单元。
    pub fn list_all(&self) -> Result<Vec<UnitEntry>> {
        systemd::list_services()
    }

    /// 启动服务。
    pub fn start(&self, unit: &str) -> Result<()> {
        systemd::validate_unit_name(unit)?;
        systemd::start(unit)
    }

    /// 停止服务。
    pub fn stop(&self, unit: &str) -> Result<()> {
        systemd::validate_unit_name(unit)?;
        systemd::stop(unit)
    }

    /// 重启服务。
    pub fn restart(&self, unit: &str) -> Result<()> {
        systemd::validate_unit_name(unit)?;
        systemd::restart(unit)
    }

    /// 启用服务。
    pub fn enable(&self, unit: &str) -> Result<()> {
        systemd::validate_unit_name(unit)?;
        systemd::enable(unit)
    }

    /// 禁用服务。
    pub fn disable(&self, unit: &str) -> Result<()> {
        systemd::validate_unit_name(unit)?;
        systemd::disable(unit)
    }

    /// 重载 systemd 单元定义。
    pub fn daemon_reload(&self) -> Result<()> {
        systemd::daemon_reload()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_units_query_missing_target_yields_nothing() {
        // 服务集合来自 systemd 而非本系统的清单，因此对不存在的 target 应返回空列表
        // 而不是报错（部署前 target 可能尚未安装）。
        let tmp = tempfile::tempdir().expect("临时目录可用");
        let config = Config {
            managed_target: "robot-system-nonexistent-test.target".to_string(),
            ..Config::default()
        };
        let manager = ServiceManager::new(Paths::with_prefix(tmp.path()), config);
        assert!(manager.target_units().expect("查询成功").is_empty());
    }

    #[test]
    fn rejects_illegal_unit_names_before_calling_systemd() {
        let tmp = tempfile::tempdir().expect("临时目录可用");
        let manager = ServiceManager::new(Paths::with_prefix(tmp.path()), Config::default());
        assert!(matches!(
            manager.start("bad; name"),
            Err(Error::InvalidArgument(_))
        ));
    }
}
