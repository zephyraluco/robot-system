//! `rsctl completions ...`：生成 shell 补全脚本（见架构文档 附录 B）。
//!
//! 补全脚本由 `clap_complete` 依据本程序的 clap 定义直接生成，因此不会与 CLI
//! 定义脱节；输出到标准输出时可直接重定向到 shell 的补全目录，例如：
//!
//! ```text
//! rsctl completions bash > /etc/bash_completion.d/rsctl
//! rsctl completions zsh  > ~/.zfunc/_rsctl
//! ```

use std::io;

use clap::CommandFactory;
use clap_complete::generate;

use crate::cli::{Cli, CompletionArgs};
use crate::error::Result;

/// 生成指定 shell 的补全脚本，写入 `--output` 文件或标准输出。
pub fn run(args: CompletionArgs) -> Result<()> {
    let mut command = Cli::command();
    // 以 clap 中声明的程序名作为补全的触发命令名，避免与二进制名分叉。
    let bin_name = command.get_name().to_string();

    match args.output {
        Some(path) => {
            let mut file = std::fs::File::create(&path)?;
            generate(args.shell, &mut command, bin_name, &mut file);
        }
        None => {
            let mut stdout = io::stdout();
            generate(args.shell, &mut command, bin_name, &mut stdout);
        }
    }
    Ok(())
}
