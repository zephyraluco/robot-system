//! DEB 软件包内的服务清单解析（见架构文档 §4.1）。
//!
//! 软件包通过 `/opt/robot-system/packages/<package>.toml` 声明它安装了哪些 systemd 服务，
//! 用于建立“软件包 → 服务”的关联。实际服务是否安装、是否启用及运行状态，仍需通过
//! systemd 与系统文件核验。

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// 单个服务的声明。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceDecl {
    /// systemd 服务单元名，例如 `robot-lidar.service`。
    pub name: String,
}

/// 一个软件包的服务清单。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageManifest {
    /// 包名。
    pub package: String,
    /// 软件包声明的版本（可选，仅供参考；实际版本以 dpkg 为准）。
    #[serde(default)]
    pub version: Option<String>,
    /// 该包依赖的其它受管软件包名。用于卸载前判断是否会破坏其它包的依赖关系。
    #[serde(default)]
    pub depends: Vec<String>,
    /// 该包提供的服务列表。
    #[serde(default)]
    pub services: Vec<ServiceDecl>,
}

impl PackageManifest {
    /// 从单个 TOML 文件加载清单。
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)?;
        let manifest: Self = toml::from_str(&text).map_err(|source| Error::Toml {
            path: path.to_path_buf(),
            source,
        })?;
        Ok(manifest)
    }

    /// 加载目录下所有清单。
    ///
    /// * 目录不存在时返回空列表。
    /// * 单个文件解析失败时**跳过**该文件，以保证 `package list` 不会因为一个损坏的
    ///   清单而整体失败；需要严格解析时应使用 [`PackageManifest::load`]。
    pub fn load_all(dir: &Path) -> Result<Vec<Self>> {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(Error::Io(err)),
        };

        let mut manifests = Vec::new();
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("toml") {
                continue;
            }
            if let Ok(manifest) = Self::load(&path) {
                manifests.push(manifest);
            }
        }
        manifests.sort_by(|a, b| a.package.cmp(&b.package));
        Ok(manifests)
    }

    /// 查找指定软件包的清单。
    pub fn find(dir: &Path, package: &str) -> Result<Option<Self>> {
        Ok(Self::load_all(dir)?
            .into_iter()
            .find(|m| m.package == package))
    }

    /// 清单声明的服务名列表。
    #[must_use]
    pub fn service_names(&self) -> Vec<String> {
        self.services.iter().map(|svc| svc.name.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_manifest_document() {
        let text = r#"
            package = "robot-lidar"
            version = "1.2.0"

            [[services]]
            name = "robot-lidar.service"

            [[services]]
            name = "robot-lidar-diagnostics.service"
        "#;
        let manifest: PackageManifest = toml::from_str(text).expect("可解析");
        assert_eq!(manifest.package, "robot-lidar");
        assert_eq!(manifest.version.as_deref(), Some("1.2.0"));
        assert_eq!(
            manifest.service_names(),
            vec!["robot-lidar.service", "robot-lidar-diagnostics.service"]
        );
    }

    #[test]
    fn version_and_services_are_optional() {
        let manifest: PackageManifest =
            toml::from_str("package = \"robot-camera\"").expect("可解析");
        assert!(manifest.version.is_none());
        assert!(manifest.services.is_empty());
    }

    #[test]
    fn missing_directory_yields_empty_list() {
        let manifests =
            PackageManifest::load_all(Path::new("/nonexistent/packages")).expect("不报错");
        assert!(manifests.is_empty());
    }

    #[test]
    fn load_all_skips_malformed_and_sorts() {
        let dir = tempfile::tempdir().expect("临时目录可用");
        std::fs::write(dir.path().join("b.toml"), "package = \"robot-b\"").expect("写入");
        std::fs::write(dir.path().join("a.toml"), "package = \"robot-a\"").expect("写入");
        std::fs::write(dir.path().join("broken.toml"), "package = ").expect("写入");
        std::fs::write(dir.path().join("ignore.txt"), "not toml").expect("写入");

        let manifests = PackageManifest::load_all(dir.path()).expect("加载成功");
        assert_eq!(manifests.len(), 2);
        assert_eq!(manifests[0].package, "robot-a");
        assert_eq!(manifests[1].package, "robot-b");
    }

    #[test]
    fn load_single_reports_error_for_broken_file() {
        let dir = tempfile::tempdir().expect("临时目录可用");
        let path = dir.path().join("broken.toml");
        std::fs::write(&path, "package = ").expect("写入");
        assert!(matches!(
            PackageManifest::load(&path),
            Err(Error::Toml { .. })
        ));
    }
}
