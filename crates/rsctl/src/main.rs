//! `rsctl`：robot-system 统一管理 CLI。
//!
//! 该程序与常驻服务 `robot-system-daemon` 是**两个相互独立的程序**（见架构文档 §1）：
//! 二者不共享 Rust 库、不通过 IPC 通信，只共享 SQLite 数据库结构以及 systemd、journald、
//! `/proc` 等系统设施。
//!
//! `rsctl` 自身实现：`PackageManager`、`DeploymentManager`、`ServiceManager`，
//! 以及对进程 / 日志 / 事件的**只读**查询。

mod app;
mod cli;
mod commands;
mod config;
mod db;
mod deployment;
mod error;
mod lock;
mod output;
mod package;
mod paths;
mod service;
mod state;
mod system;

use std::process::ExitCode;

use clap::Parser;

use crate::cli::Cli;

fn main() -> ExitCode {
    let cli = Cli::parse();

    match app::run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            // 机器可读模式下的错误同样以 JSON 形式输出，便于上层程序解析。
            if std::env::args().any(|a| a == "--json") {
                let payload = serde_json::json!({ "error": err.to_string() });
                eprintln!("{payload}");
            } else {
                eprintln!("错误：{err}");
            }
            ExitCode::FAILURE
        }
    }
}
