use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{Shell, generate};
use std::io;

#[derive(Debug, Parser)]
#[command(name = "rsctl", about = "Control the robot system")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Show the robot system status.
    Status,
    /// Restart the robot system.
    Restart,
    /// Stop the robot system.
    Stop,
    /// Start the robot system.
    Start,
    /// Generate shell completion scripts.
    #[command(hide = true)]
    Completions {
        #[arg(value_enum)]
        shell: Shell,
    },
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Command::Status => println!("status"),
        Command::Restart => println!("restart"),
        Command::Stop => println!("stop"),
        Command::Start => println!("start"),
        Command::Completions { shell } => {
            generate(shell, &mut Cli::command(), "rsctl", &mut io::stdout());
        }
    }
}
