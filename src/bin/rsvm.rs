use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{Shell, generate};
use std::io;

#[derive(Debug, Parser)]
#[command(name = "rsvm", about = "Manage the robot system")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Install the robot system.
    Install,
    /// List available or installed components.
    List,
    /// Upgrade the robot system.
    Upgrade,
    /// Initialize the robot system.
    Init,
    /// Show information about the robot system.
    Info,
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
        Command::Install => println!("install"),
        Command::List => println!("list"),
        Command::Upgrade => println!("upgrade"),
        Command::Init => println!("init"),
        Command::Info => println!("info"),
        Command::Completions { shell } => {
            generate(shell, &mut Cli::command(), "rsvm", &mut io::stdout());
        }
    }
}