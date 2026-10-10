//! 应用入口：装配上下文并分发子命令。

use crate::cli::{Cli, Command};
use crate::commands;
use crate::config::Config;
use crate::output::Format;
use crate::paths::Paths;

/// 命令执行上下文。
#[derive(Debug)]
pub struct Context {
    /// 文件系统布局。
    pub paths: Paths,
    /// 运行时配置。
    pub config: Config,
    /// 输出格式。
    pub format: Format,
}

/// 解析参数、构建上下文并执行命令。
pub fn run(cli: Cli) -> anyhow::Result<()> {
    let mut paths = Paths::detect();
    if let Some(root) = cli.opt_root {
        paths.set_opt_root(root);
    }
    let config = Config::load(&paths.config_file())?;
    let ctx = Context {
        paths,
        config,
        format: Format::from_flag(cli.json),
    };

    match cli.command {
        Command::Package(command) => commands::package::run(&ctx, command)?,
        Command::System(command) => commands::system::run(&ctx, command)?,
        Command::Service(command) => commands::service::run(&ctx, command)?,
        Command::Process(command) => commands::process::run(&ctx, command)?,
        Command::Metrics(args) => commands::metrics::run(&ctx, args)?,
        Command::Error(command) => commands::events::run(&ctx, command)?,
        Command::Logs(args) => commands::logs::run(&ctx, args)?,
        Command::Completions(args) => commands::completion::run(args)?,
    }
    Ok(())
}
