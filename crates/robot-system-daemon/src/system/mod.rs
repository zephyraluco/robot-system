//! 系统适配层：systemd、journald、`/proc`（见架构文档 §3）。
//!
//! 这是常驻服务**自己实现**的一份，与 `rsctl` 的同名模块互不共享代码（见 §2.3）。

pub mod journald;
pub mod procfs;
pub mod systemd;

use std::ffi::OsStr;
use std::process::{Command, Stdio};

use crate::error::{Error, Result};

/// 命令执行结果。
#[derive(Debug)]
pub struct CommandOutput {
    /// 退出状态。
    pub status: std::process::ExitStatus,
    /// 标准输出。
    pub stdout: String,
    /// 标准错误输出。
    pub stderr: String,
}

impl CommandOutput {
    /// 命令是否成功退出。
    #[must_use]
    pub fn success(&self) -> bool {
        self.status.success()
    }

    /// 输出行迭代器。
    pub fn lines(&self) -> impl Iterator<Item = &str> {
        self.stdout
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
    }

    /// 退出状态描述。
    #[must_use]
    pub fn status_text(&self) -> String {
        self.status
            .code()
            .map_or_else(|| "signal".to_string(), |code| code.to_string())
    }
}

/// 使用参数数组执行命令，**不经过 shell**（见 §13.2）。
pub fn run<S: AsRef<OsStr>>(program: &str, args: &[S]) -> Result<CommandOutput> {
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .env("LANG", "C")
        .env("LC_ALL", "C")
        .output()
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => Error::CommandNotFound {
                program: program.to_string(),
            },
            _ => Error::Io(err),
        })?;

    Ok(CommandOutput {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

/// 执行命令，非零退出码时返回错误。
pub fn run_checked<S: AsRef<OsStr>>(program: &str, args: &[S]) -> Result<CommandOutput> {
    let output = run(program, args)?;
    if !output.success() {
        return Err(Error::CommandFailed {
            program: program.to_string(),
            status: output.status_text(),
            stderr: output.stderr.trim().to_string(),
        });
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_stdout_and_status() {
        let out = run("echo", &["hi"]).expect("echo 可用");
        assert!(out.success());
        assert_eq!(out.stdout.trim(), "hi");
    }

    #[test]
    fn reports_missing_program() {
        let err = run("definitely-not-real-xyz", &[] as &[&str]).expect_err("应报错");
        assert!(matches!(err, Error::CommandNotFound { .. }));
    }
}
