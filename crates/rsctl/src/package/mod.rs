//! `PackageManager`：软件包管理（见架构文档 §3.1）。
//!
//! 职责：查询已安装的自定义 DEB 包、安装 / 升级 / 卸载、读取服务声明、管理来源与
//! 摘要信息，并识别包与服务之间的关系。
//!
//! 实现要点：
//! * 通过参数数组调用系统工具，**禁止**把用户输入拼成 Shell 命令；
//! * 将系统包查询结果与自身状态（清单文件）进行对账；
//! * 以 dpkg 的实际状态作为安装状态的依据，不单独依赖清单或内部记录。

pub mod manifest;

use std::collections::BTreeSet;
use std::path::Path;

use serde::Serialize;

use crate::error::{Error, Result};
use crate::paths::Paths;
use crate::system::dpkg::{self, InstalledPackage};

pub use manifest::PackageManifest;

/// 本地 DEB 文件的描述信息。
#[derive(Debug, Clone, Serialize)]
pub struct DebInfo {
    /// 文件路径。
    pub path: String,
    /// 包名。
    pub package: String,
    /// 版本。
    pub version: String,
    /// 架构。
    pub architecture: String,
    /// SHA-256 摘要。
    pub sha256: String,
}

/// 一个受管软件包的综合视图（清单声明 + dpkg 实际状态）。
#[derive(Debug, Clone, Serialize)]
pub struct ManagedPackage {
    /// 包名。
    pub name: String,
    /// 是否已安装（以 dpkg 为准）。
    pub installed: bool,
    /// dpkg 报告的已安装版本。
    pub installed_version: Option<String>,
    /// dpkg 报告的架构。
    pub architecture: Option<String>,
    /// 清单声明的版本（仅供参考）。
    pub declared_version: Option<String>,
    /// 清单声明的服务列表。
    pub services: Vec<String>,
    /// 清单文件路径。
    pub manifest_path: Option<String>,
}

/// 软件包管理器。
#[derive(Debug, Clone)]
pub struct PackageManager {
    paths: Paths,
}

impl PackageManager {
    /// 构造软件包管理器。
    #[must_use]
    pub fn new(paths: Paths) -> Self {
        Self { paths }
    }

    /// 所有软件包清单。
    pub fn manifests(&self) -> Result<Vec<PackageManifest>> {
        PackageManifest::load_all(&self.paths.packages_dir())
    }

    /// 指定软件包的清单。
    pub fn manifest_for(&self, package: &str) -> Result<Option<PackageManifest>> {
        PackageManifest::find(&self.paths.packages_dir(), package)
    }

    /// 查询单个软件包的 dpkg 状态。
    pub fn installed_one(&self, name: &str) -> Result<Option<InstalledPackage>> {
        dpkg::query(name)
    }

    /// 列出所有受管软件包（拥有清单的包），并与 dpkg 状态对账。
    pub fn managed_packages(&self) -> Result<Vec<ManagedPackage>> {
        let mut result = Vec::new();
        for manifest in self.manifests()? {
            let manifest_path = self
                .paths
                .packages_dir()
                .join(format!("{}.toml", manifest.package))
                .display()
                .to_string();
            let installed = dpkg::query(&manifest.package)?;
            result.push(ManagedPackage {
                name: manifest.package.clone(),
                installed: installed
                    .as_ref()
                    .is_some_and(InstalledPackage::is_installed),
                installed_version: installed
                    .as_ref()
                    .filter(|pkg| pkg.is_installed())
                    .map(|pkg| pkg.version.clone()),
                architecture: installed.as_ref().map(|pkg| pkg.architecture.clone()),
                declared_version: manifest.version.clone(),
                services: manifest.service_names(),
                manifest_path: Some(manifest_path),
            });
        }
        Ok(result)
    }

    /// 查询单个受管软件包的详情。
    ///
    /// 若该包没有清单（未被本系统管理），返回 [`Error::InvalidArgument`]。
    pub fn package_info(&self, name: &str) -> Result<ManagedPackage> {
        validate_package_name(name)?;
        let manifest = self.manifest_for(name)?.ok_or_else(|| {
            Error::InvalidArgument(format!(
                "软件包 `{name}` 未被 robot-system 管理（缺少服务清单）"
            ))
        })?;
        let installed = dpkg::query(name)?;
        let manifest_path = self
            .paths
            .packages_dir()
            .join(format!("{name}.toml"))
            .display()
            .to_string();
        Ok(ManagedPackage {
            name: manifest.package.clone(),
            installed: installed
                .as_ref()
                .is_some_and(InstalledPackage::is_installed),
            installed_version: installed
                .as_ref()
                .filter(|pkg| pkg.is_installed())
                .map(|pkg| pkg.version.clone()),
            architecture: installed.as_ref().map(|pkg| pkg.architecture.clone()),
            declared_version: manifest.version.clone(),
            services: manifest.service_names(),
            manifest_path: Some(manifest_path),
        })
    }

    /// 清单声明的服务名列表；无清单时返回空。
    pub fn services_of(&self, package: &str) -> Result<Vec<String>> {
        Ok(self
            .manifest_for(package)?
            .map_or_else(Vec::new, |m| m.service_names()))
    }

    /// 查找除 `exclude` 之外、同样声明了该服务的其它软件包。
    ///
    /// 用于卸载时判断服务是否为共享服务：共享服务不应因为某个包被卸载而停止。
    pub fn other_owners_of_service(&self, service: &str, exclude: &str) -> Result<Vec<String>> {
        let mut owners = BTreeSet::new();
        for manifest in self.manifests()? {
            if manifest.package == exclude {
                continue;
            }
            if manifest.services.iter().any(|decl| decl.name == service) {
                owners.insert(manifest.package.clone());
            }
        }
        Ok(owners.into_iter().collect())
    }

    /// 查找声明依赖 `package` 的其它受管软件包。
    ///
    /// 用于卸载前的依赖检查（见 §7.4）。
    pub fn dependents_of(&self, package: &str) -> Result<Vec<String>> {
        let mut dependents = BTreeSet::new();
        for manifest in self.manifests()? {
            if manifest.package == package {
                continue;
            }
            if manifest.depends.iter().any(|dep| dep == package) {
                dependents.insert(manifest.package.clone());
            }
        }
        Ok(dependents.into_iter().collect())
    }

    /// 读取本地 DEB 文件的元数据与摘要。
    pub fn describe_deb(&self, path: &Path) -> Result<DebInfo> {
        if !path.is_file() {
            return Err(Error::InvalidArgument(format!(
                "DEB 文件不存在：{}",
                path.display()
            )));
        }
        let package = dpkg::deb_field(path, "Package")?;
        let version = dpkg::deb_field(path, "Version")?;
        let architecture = dpkg::deb_field(path, "Architecture")?;
        let sha256 = dpkg::sha256_file(path)?;
        validate_package_name(&package)?;
        Ok(DebInfo {
            path: path.display().to_string(),
            package,
            version,
            architecture,
            sha256,
        })
    }

    /// 安装（或升级）本地 DEB 文件。
    pub fn install_deb(&self, path: &Path) -> Result<()> {
        dpkg::install_local(path)
    }

    /// 卸载软件包。
    pub fn remove(&self, name: &str) -> Result<()> {
        validate_package_name(name)?;
        dpkg::remove(name)
    }

    /// 本机架构。
    pub fn host_architecture(&self) -> Result<String> {
        dpkg::host_architecture()
    }
}

/// 校验 dpkg 包名是否合法（小写字母、数字及 `+ - .`）。
pub fn validate_package_name(name: &str) -> Result<()> {
    let valid = !name.is_empty()
        && name.len() <= 255
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '+' | '-' | '.'));

    if valid {
        Ok(())
    } else {
        Err(Error::InvalidArgument(format!("非法软件包名：{name}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_package_names() {
        assert!(validate_package_name("robot-lidar").is_ok());
        assert!(validate_package_name("robot-lidar2").is_ok());
        assert!(validate_package_name("a+b.c").is_ok());
    }

    #[test]
    fn rejects_invalid_package_names() {
        assert!(validate_package_name("").is_err());
        assert!(validate_package_name("Robot-Lidar").is_err());
        assert!(validate_package_name("-leading").is_err());
        assert!(validate_package_name("robot lidar").is_err());
        assert!(validate_package_name("robot;rm -rf /").is_err());
    }

    #[test]
    fn managed_packages_reflect_manifest_and_dpkg() {
        let tmp = tempfile::tempdir().expect("临时目录可用");
        let paths = Paths::with_prefix(tmp.path());
        std::fs::create_dir_all(paths.packages_dir()).expect("创建目录");
        std::fs::write(
            paths.packages_dir().join("bash.toml"),
            "package = \"bash\"\n\n[[services]]\nname = \"bash.service\"\n",
        )
        .expect("写入清单");

        let manager = PackageManager::new(paths);
        let packages = manager.managed_packages().expect("查询成功");
        assert_eq!(packages.len(), 1);
        let pkg = &packages[0];
        assert_eq!(pkg.name, "bash");
        assert_eq!(pkg.services, vec!["bash.service"]);
        // 测试机上的 bash 通常已安装，但不做硬性假设。
        if pkg.installed {
            assert!(pkg.installed_version.is_some());
        }
    }

    #[test]
    fn other_owners_detect_shared_services() {
        let tmp = tempfile::tempdir().expect("临时目录可用");
        let paths = Paths::with_prefix(tmp.path());
        std::fs::create_dir_all(paths.packages_dir()).expect("创建目录");
        for (pkg, service) in [("robot-a", "shared.service"), ("robot-b", "shared.service")] {
            std::fs::write(
                paths.packages_dir().join(format!("{pkg}.toml")),
                format!("package = \"{pkg}\"\n\n[[services]]\nname = \"{service}\"\n"),
            )
            .expect("写入清单");
        }

        let manager = PackageManager::new(paths);
        let owners = manager
            .other_owners_of_service("shared.service", "robot-a")
            .expect("查询成功");
        assert_eq!(owners, vec!["robot-b"]);
    }

    #[test]
    fn unmanaged_package_info_is_rejected() {
        let tmp = tempfile::tempdir().expect("临时目录可用");
        let manager = PackageManager::new(Paths::with_prefix(tmp.path()));
        assert!(matches!(
            manager.package_info("bash"),
            Err(Error::InvalidArgument(_))
        ));
    }

    #[test]
    fn dependents_are_detected() {
        let tmp = tempfile::tempdir().expect("临时目录可用");
        let paths = Paths::with_prefix(tmp.path());
        std::fs::create_dir_all(paths.packages_dir()).expect("创建目录");
        std::fs::write(
            paths.packages_dir().join("robot-core.toml"),
            "package = \"robot-core\"\n",
        )
        .expect("写入");
        std::fs::write(
            paths.packages_dir().join("robot-app.toml"),
            "package = \"robot-app\"\ndepends = [\"robot-core\"]\n",
        )
        .expect("写入");

        let manager = PackageManager::new(paths);
        assert_eq!(
            manager.dependents_of("robot-core").expect("查询成功"),
            vec!["robot-app"]
        );
        assert!(
            manager
                .dependents_of("robot-app")
                .expect("查询成功")
                .is_empty()
        );
    }
}
