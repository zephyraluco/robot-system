//! 安全的外部命令执行封装。
//!
//! 安全要求：使用 `Command` 参数数组调用系统接口，**禁止**通过
//! `sh -c` 拼接用户输入。本模块只接受“程序名 + 参数数组”，不做任何字符串拼装。

use std::ffi::OsStr;
use std::process::{Command, Stdio};

use crate::error::{Error, Result};

/// 命令执行结果。
#[derive(Debug)]
pub struct CommandOutput {
    /// 退出状态。
    pub status: std::process::ExitStatus,
    /// 标准输出（已按 UTF-8 有损解码）。
    pub stdout: String,
    /// 标准错误输出（已按 UTF-8 有损解码）。
    pub stderr: String,
}

impl CommandOutput {
    /// 命令是否成功退出。
    #[must_use]
    pub fn success(&self) -> bool {
        self.status.success()
    }

    /// 标准输出的非空行迭代器（已去除首尾空白）。
    pub fn lines(&self) -> impl Iterator<Item = &str> {
        self.stdout
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
    }

    /// 退出状态的文字描述（退出码或信号）。
    #[must_use]
    pub fn status_text(&self) -> String {
        match self.status.code() {
            Some(code) => code.to_string(),
            None => "signal".to_string(),
        }
    }
}

/// 使用参数数组执行命令，**不经过 shell**。
///
/// 环境变量 `LANG` / `LC_ALL` 被固定为 `C`，以保证输出可稳定解析。
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

/// 执行命令；若退出码非零则返回 [`Error::CommandFailed`]。
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

/// 命令是否存在于 `PATH` 中。
#[must_use]
pub fn exists(program: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        let candidate = dir.join(program);
        candidate.is_file()
    })
}

/// 当前进程是否以 root 身份运行。
#[must_use]
pub fn is_root() -> bool {
    // SAFETY: `geteuid` 无参数、无副作用，始终安全。
    unsafe { libc::geteuid() == 0 }
}

/// 要求当前进程具备 root 权限，否则返回 [`Error::PermissionDenied`]。
pub fn require_root(action: &str) -> Result<()> {
    if is_root() {
        Ok(())
    } else {
        Err(Error::PermissionDenied(action.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_captures_stdout() {
        let out = run("echo", &["hello"]).expect("echo 可用");
        assert!(out.success());
        assert_eq!(out.stdout.trim(), "hello");
    }

    #[test]
    fn run_checked_reports_failure() {
        let err = run_checked("false", &[] as &[&str]).expect_err("false 应返回失败");
        assert!(matches!(err, Error::CommandFailed { .. }));
    }

    #[test]
    fn run_reports_missing_program() {
        let err = run("definitely-not-a-real-binary-xyz", &[] as &[&str])
            .expect_err("不存在的命令应报错");
        assert!(matches!(err, Error::CommandNotFound { .. }));
    }

    #[test]
    fn exists_detects_common_tools() {
        assert!(exists("sh"));
        assert!(!exists("definitely-not-a-real-binary-xyz"));
    }
}
