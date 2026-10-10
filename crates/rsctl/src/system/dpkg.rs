//! dpkg / APT 适配层。
//!
//! Ubuntu 的 dpkg 可以安装和管理 DEB，但不是完整的依赖求解器（见 §5.1）。
//! 因此在存在依赖时优先使用受控 APT 的依赖解析能力，dpkg 作为回退。

use std::path::Path;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::system::command;

/// 已安装软件包的简要信息（来自 `dpkg-query`）。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct InstalledPackage {
    /// 包名。
    pub name: String,
    /// 版本。
    pub version: String,
    /// 架构。
    pub architecture: String,
    /// dpkg 状态串，例如 `install ok installed`。
    pub status: String,
}

impl InstalledPackage {
    /// dpkg 状态是否表示“已正确安装”。
    #[must_use]
    pub fn is_installed(&self) -> bool {
        self.status == "install ok installed"
    }
}

/// `dpkg-query` 使用的字段格式串（用制表符与换行分隔，便于稳定解析）。
const QUERY_FORMAT: &str = "${Package}\\t${Version}\\t${Architecture}\\t${Status}\\n";

/// 查询单个软件包的安装信息。
pub fn query(name: &str) -> Result<Option<InstalledPackage>> {
    let out = command::run("dpkg-query", &["-W", "-f", QUERY_FORMAT, name])?;
    if !out.success() {
        // dpkg-query 对未安装的包返回非零；区分“未安装”与“其它错误”。
        if out.stderr.contains("no packages found") || out.stderr.contains("is not installed") {
            return Ok(None);
        }
        return Err(Error::CommandFailed {
            program: "dpkg-query".to_string(),
            status: out.status_text(),
            stderr: out.stderr.trim().to_string(),
        });
    }
    Ok(parse_query_output(&out.stdout).into_iter().next())
}

/// 读取本地 DEB 文件的控制字段。
pub fn deb_field(deb: &Path, field: &str) -> Result<String> {
    let out = command::run_checked("dpkg-deb", &["-f", path_str(deb)?, field])?;
    Ok(out.stdout.trim().to_string())
}

/// 计算文件的 SHA-256 摘要。
pub fn sha256_file(path: &Path) -> Result<String> {
    use std::io::Read;

    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(to_hex(hasher.finalize().as_slice()))
}

/// 将摘要字节编码为小写十六进制字符串。
///
/// sha2 0.11 起的 `finalize()` 返回 `Array<u8, …>`，不再直接实现 `LowerHex`，
/// 因此在这里显式编码，避免为这一个用途引入额外的 hex 依赖。
fn to_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(DIGITS[usize::from(byte >> 4)]));
        out.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    out
}

/// 返回本机 dpkg 架构（例如 `amd64`、`arm64`）。
pub fn host_architecture() -> Result<String> {
    let out = command::run_checked("dpkg", &["--print-architecture"])?;
    let arch = out.stdout.trim();
    if arch.is_empty() {
        return Err(Error::ParseOutput {
            program: "dpkg --print-architecture".to_string(),
            message: "输出为空".to_string(),
        });
    }
    Ok(arch.to_string())
}

/// 安装或升级本地 DEB 文件。
///
/// 优先使用受控 APT 解析依赖；若系统没有 `apt-get`，退回到 `dpkg -i`。
pub fn install_local(deb: &Path) -> Result<()> {
    let deb = canonicalize(deb)?;
    if command::exists("apt-get") {
        command::run_checked(
            "apt-get",
            &[
                "install",
                "-y",
                "--no-install-recommends",
                "-o",
                "Dpkg::Options::=--force-confold",
                path_str(&deb)?,
            ],
        )?;
    } else if command::exists("dpkg") {
        command::run_checked("dpkg", &["-i", path_str(&deb)?])?;
    } else {
        return Err(Error::Unsupported(
            "系统缺少 apt-get 与 dpkg，无法安装 DEB".to_string(),
        ));
    }
    Ok(())
}

/// 移除软件包（保留配置文件）；优先使用 APT，回退到 dpkg。
pub fn remove(name: &str) -> Result<()> {
    if command::exists("apt-get") {
        command::run_checked("apt-get", &["remove", "-y", name])?;
    } else if command::exists("dpkg") {
        command::run_checked("dpkg", &["-r", name])?;
    } else {
        return Err(Error::Unsupported(
            "系统缺少 apt-get 与 dpkg，无法卸载软件包".to_string(),
        ));
    }
    Ok(())
}

/// 用 dpkg 的版本比较算法判断 `a <op> b` 是否成立。
///
/// `op` 取值与 `dpkg --compare-versions` 一致，例如 `lt`、`le`、`eq`、`gt`、`ge`。
pub fn compare_versions(a: &str, op: &str, b: &str) -> Result<bool> {
    const VALID_OPS: [&str; 5] = ["lt", "le", "eq", "ge", "gt"];
    if !VALID_OPS.contains(&op) {
        return Err(Error::InvalidArgument(format!("非法版本比较操作符：{op}")));
    }
    let out = command::run("dpkg", &["--compare-versions", a, op, b])?;
    match out.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(Error::CommandFailed {
            program: "dpkg".to_string(),
            status: out.status_text(),
            stderr: out.stderr.trim().to_string(),
        }),
    }
}

/// 将路径转换为 `&str`；非 UTF-8 路径视为非法参数。
fn path_str(path: &Path) -> Result<&str> {
    path.to_str()
        .ok_or_else(|| Error::InvalidArgument(format!("路径不是合法 UTF-8：{}", path.display())))
}

/// 规范化 DEB 路径为绝对路径。
fn canonicalize(path: &Path) -> Result<std::path::PathBuf> {
    std::fs::canonicalize(path).map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            Error::InvalidArgument(format!("DEB 文件不存在：{}", path.display()))
        } else {
            Error::Io(err)
        }
    })
}

/// 解析 `dpkg-query` 的制表符分隔输出。
fn parse_query_output(text: &str) -> Vec<InstalledPackage> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.splitn(4, '\t');
            let name = fields.next()?.trim();
            let version = fields.next()?.trim();
            let architecture = fields.next()?.trim();
            let status = fields.next().unwrap_or("").trim();
            if name.is_empty() {
                return None;
            }
            Some(InstalledPackage {
                name: name.to_string(),
                version: version.to_string(),
                architecture: architecture.to_string(),
                status: status.to_string(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tab_separated_query_output() {
        let text = "bash\t5.2.21-2ubuntu4\tamd64\tinstall ok installed\n\
                    curl\t8.5.0-2ubuntu10\tamd64\tdesired ok not-installed\n";
        let packages = parse_query_output(text);
        assert_eq!(packages.len(), 2);
        assert_eq!(packages[0].name, "bash");
        assert!(packages[0].is_installed());
        assert!(!packages[1].is_installed());
    }

    #[test]
    fn skips_blank_lines() {
        let packages = parse_query_output("\n\n");
        assert!(packages.is_empty());
    }

    #[test]
    fn sha256_matches_known_vector() {
        let dir = tempfile::tempdir().expect("临时目录可用");
        let file = dir.path().join("data");
        std::fs::write(&file, b"abc").expect("写入成功");
        // `echo -n abc | sha256sum` 的已知结果。
        assert_eq!(
            sha256_file(&file).expect("计算成功"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn missing_deb_is_invalid_argument() {
        let err = install_local(Path::new("/nonexistent/foo.deb")).expect_err("应报错");
        assert!(matches!(err, Error::InvalidArgument(_)));
    }

    #[test]
    fn compare_versions_uses_dpkg_ordering() {
        assert!(compare_versions("1.0.0", "lt", "1.2.0").expect("比较成功"));
        assert!(!compare_versions("2.0.0", "lt", "1.2.0").expect("比较成功"));
        assert!(compare_versions("1.2.0", "eq", "1.2.0").expect("比较成功"));
        assert!(matches!(
            compare_versions("1.0.0", "wat", "1.0.0"),
            Err(Error::InvalidArgument(_))
        ));
    }
}
