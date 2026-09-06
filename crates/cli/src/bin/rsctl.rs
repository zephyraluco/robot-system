use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{Shell, generate};
use std::io;
use std::process::Command as ProcessCommand;

#[derive(Debug, Parser)]
#[command(name = "rsctl", about = "Control the robot system")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Show the robot system status.
    Status {
        #[arg(value_name = "PKG")]
        pkg: String,
    },
    /// Restart the robot system.
    Restart {
        #[arg(value_name = "PKG")]
        pkg: String,
    },
    /// Stop the robot system.
    Stop {
        #[arg(value_name = "PKG")]
        pkg: String,
    },
    /// Start the robot system.
    Start {
        #[arg(value_name = "PKG")]
        pkg: String,
    },
    /// Generate shell completion scripts.
    #[command(hide = true)]
    Completions {
        #[arg(value_enum)]
        shell: Shell,
    },
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Command::Status { pkg } => control_service("status", &pkg)?,
        Command::Restart { pkg } => control_service("restart", &pkg)?,
        Command::Stop { pkg } => control_service("stop", &pkg)?,
        Command::Start { pkg } => control_service("start", &pkg)?,
        Command::Completions { shell } => {
            generate(shell, &mut Cli::command(), "rsctl", &mut io::stdout());
        }
    }

    Ok(())
}

fn control_service(action: &str, pkg: &str) -> Result<(), Box<dyn std::error::Error>> {
    let service = format!("{pkg}.service");
    let status = ProcessCommand::new("systemctl")
        .arg(action)
        .arg(&service)
        .status()?;

    if !status.success() {
        return Err(format!("systemctl {action} failed for {service}").into());
    }

    Ok(())
}
