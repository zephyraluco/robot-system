//! 命令行接口定义（见架构文档 附录 B）。

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

/// robot-system 统一管理 CLI。
#[derive(Debug, Parser)]
#[command(
    name = "rsctl",
    version,
    about = "robot-system 统一管理 CLI：软件包、部署、服务与运行状态",
    long_about = None,
    propagate_version = true
)]
pub struct Cli {
    /// 以 JSON 输出结果，便于脚本与上层程序调用。
    #[arg(long, global = true)]
    pub json: bool,

    /// 覆盖程序与静态资源根目录（默认 `/opt/robot-system`）。
    #[arg(long, global = true, value_name = "DIR")]
    pub opt_root: Option<PathBuf>,

    /// 子命令。
    #[command(subcommand)]
    pub command: Command,
}

/// 顶层子命令。
#[derive(Debug, Subcommand)]
pub enum Command {
    /// 软件包管理与部署任务。
    #[command(subcommand)]
    Package(PackageCommand),

    /// 统一 target 与系统管理状态。
    #[command(subcommand)]
    System(SystemCommand),

    /// systemd 服务生命周期管理。
    #[command(subcommand)]
    Service(ServiceCommand),

    /// 进程运行实例。
    #[command(subcommand)]
    Process(ProcessCommand),

    /// 查询服务资源指标。
    Metrics(MetricsArgs),

    /// 查询运行类错误事件。
    #[command(subcommand)]
    Error(ErrorCommand),

    /// 查询服务原始日志。
    Logs(LogsArgs),

    /// 生成 shell 补全脚本。
    #[command(hide = true)]
    Completions(CompletionArgs),
}

/// 软件包与部署任务子命令。
#[derive(Debug, Subcommand)]
pub enum PackageCommand {
    /// 列出受管软件包。
    List,
    /// 查询软件包详情。
    Info {
        /// 包名。
        name: String,
    },
    /// 安装本地 DEB。
    Install {
        /// DEB 文件路径。
        deb: PathBuf,
    },
    /// 升级到指定 DEB。
    Upgrade {
        /// DEB 文件路径。
        deb: PathBuf,
    },
    /// 卸载软件包。
    Remove {
        /// 包名。
        name: String,
    },
    /// 部署任务查询。
    #[command(subcommand)]
    Task(TaskCommand),
    /// 查询指定软件包的部署历史。
    History {
        /// 包名。
        name: String,
    },
}

/// 部署任务子命令。
#[derive(Debug, Subcommand)]
pub enum TaskCommand {
    /// 列出部署任务。
    List,
    /// 查询指定任务详情（含步骤记录）。
    Show {
        /// 任务 ID。
        id: String,
    },
}

/// 系统子命令。
#[derive(Debug, Subcommand)]
pub enum SystemCommand {
    /// 查看受管 target 与服务汇总状态。
    Status,
}

/// 服务子命令。
#[derive(Debug, Subcommand)]
pub enum ServiceCommand {
    /// 列出受管服务及状态。
    List {
        /// 列出系统中所有 systemd service 单元，而不只是本系统清单声明的服务。
        #[arg(long)]
        all: bool,
    },
    /// 查询单个服务详情。
    Status {
        /// systemd 单元名，例如 `robot-lidar.service`。
        unit: String,
    },
    /// 启动服务。
    Start {
        /// systemd 单元名。
        unit: String,
    },
    /// 停止服务。
    Stop {
        /// systemd 单元名。
        unit: String,
    },
    /// 重启服务。
    Restart {
        /// systemd 单元名。
        unit: String,
    },
    /// 启用服务（开机自启）。
    Enable {
        /// systemd 单元名。
        unit: String,
    },
    /// 禁用服务。
    Disable {
        /// systemd 单元名。
        unit: String,
    },
}

/// 进程子命令。
#[derive(Debug, Subcommand)]
pub enum ProcessCommand {
    /// 查看当前运行实例。
    List {
        /// 最多返回的记录数（仅历史模式生效）。
        #[arg(long, default_value_t = 50)]
        limit: u32,
    },
    /// 查看指定服务的运行历史。
    History {
        /// systemd 单元名。
        unit: String,
        /// 最多返回的记录数。
        #[arg(long, default_value_t = 50)]
        limit: u32,
    },
}

/// 指标查询参数。
#[derive(Debug, Args)]
pub struct MetricsArgs {
    /// systemd 单元名。
    pub unit: String,
    /// 最多返回的采样点数。
    #[arg(long, default_value_t = 20)]
    pub limit: u32,
}

/// 错误事件子命令。
#[derive(Debug, Subcommand)]
pub enum ErrorCommand {
    /// 列出错误事件。
    List {
        /// 按事件类型过滤，例如 `process_exited_abnormally`。
        #[arg(long)]
        event_type: Option<String>,
        /// 按对象（服务名）过滤。
        #[arg(long)]
        service: Option<String>,
        /// 最多返回的记录数。
        #[arg(long, default_value_t = 50)]
        limit: u32,
    },
}

/// 日志查询参数。
#[derive(Debug, Args)]
pub struct LogsArgs {
    /// systemd 单元名。
    pub unit: String,
    /// 起始时间，支持 `10m` / `2h` / `1d` 或 journalctl 原生写法。
    #[arg(long)]
    pub since: Option<String>,
    /// 优先级过滤，例如 `err`、`warning`。
    #[arg(long)]
    pub priority: Option<String>,
    /// 最多返回的日志条数。
    #[arg(long, default_value_t = 50)]
    pub lines: usize,
}

/// 补全脚本生成参数。
#[derive(Debug, Args)]
pub struct CompletionArgs {
    /// 目标 shell（`bash`、`zsh`、`fish`、`elvish`、`powershell`）。
    #[arg(value_enum, value_name = "SHELL")]
    pub shell: clap_complete::Shell,
    /// 输出文件；省略时写入标准输出，便于重定向到补全目录。
    #[arg(long, short, value_name = "FILE")]
    pub output: Option<PathBuf>,
}
