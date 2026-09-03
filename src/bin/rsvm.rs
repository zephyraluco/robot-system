use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{Shell, generate};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Command as ProcessCommand;

const CONFIG_PATH: &str = "/etc/robot-system/robot-system.conf";
const PACKAGE_DIR: &str = "/tmp/rs-pkgs";

#[derive(Debug, Default, Deserialize, Serialize)]
struct Config {
    #[serde(default)]
    normal: NormalConfig,
    #[serde(default)]
    ftp: FtpConfig,
    #[serde(default)]
    pkg: BTreeMap<String, String>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct NormalConfig {
    #[serde(default)]
    version: String,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default)]
struct FtpConfig {
    host: String,
    port: u16,
    username: String,
    password: String,
    remote_path: String,
}

#[derive(Debug, Parser)]
#[command(name = "rsvm", about = "Manage the robot system")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Install the robot system.
    Install {
        #[arg(value_name = "PKG")]
        pkg: String,
        #[arg(value_name = "VERSION")]
        version: String,
    },
    /// List available or installed components.
    List,
    /// Show the robot system history for a package.
    History {
        #[arg(value_name = "PKG")]
        pkg: String,
    },
    /// Reload the robot system.
    Reload,
    /// Initialize the robot system.
    Init,
    /// Show information about the robot system.
    Info {
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
        Command::Install { pkg, version } => install_package(&pkg, &version)?,
        Command::List => list_packages()?,
        Command::History { pkg } => show_history(&pkg)?,
        Command::Reload => reload_packages()?,
        Command::Init => {
            ensure_init_runs_as_root()?;
            initialize_config()?
        }
        Command::Info { pkg } => show_package_info(&pkg)?,
        Command::Completions { shell } => {
            generate(shell, &mut Cli::command(), "rsvm", &mut io::stdout());
        }
    }

    Ok(())
}

#[cfg(unix)]
fn ensure_init_runs_as_root() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var_os("SUDO_USER").is_some() {
        return Ok(());
    }

    let executable = std::env::current_exe()?;
    let error = ProcessCommand::new("sudo")
        .arg("--")
        .arg(executable)
        .args(std::env::args_os().skip(1))
        .exec();
    Err(error.into())
}

#[cfg(not(unix))]
fn ensure_init_runs_as_root() -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}

fn read_config() -> Result<Config, Box<dyn std::error::Error>> {
    let content = fs::read_to_string(CONFIG_PATH)?;
    Ok(toml::from_str(&content)?)
}

fn list_packages() -> Result<(), Box<dyn std::error::Error>> {
    let config = read_config()?;

    println!("robot-system({}) current status:", config.normal.version);
    for (pkg, version) in config.pkg {
        let status = match installed_package_version(&pkg)? {
            Some(installed_version) if installed_version == version => "\x1b[32mOK\x1b[0m",
            Some(_) => "\x1b[33mMISMATCH\x1b[0m",
            None => "\x1b[31mNone\x1b[0m",
        };
        println!("    {pkg:<25} {version:<15} --> {status}");
    }

    Ok(())
}

fn show_package_info(pkg: &str) -> Result<(), Box<dyn std::error::Error>> {
    let status = ProcessCommand::new("dpkg")
        .args(["--status", pkg])
        .status()?;
    if !status.success() {
        return Err(format!("failed to get information for {pkg}").into());
    }

    Ok(())
}

fn reload_packages() -> Result<(), Box<dyn std::error::Error>> {
    let services_path = PathBuf::from(CONFIG_PATH)
        .parent()
        .ok_or("configuration directory not found")?
        .join("services");
    let mut config = read_config()?;

    for entry in fs::read_dir(services_path)? {
        let path = entry?.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("service") {
            continue;
        }

        let Some(pkg) = path.file_stem().and_then(|name| name.to_str()) else {
            continue;
        };
        match installed_package_version(pkg)? {
            Some(version) => {
                let is_new = !config.pkg.contains_key(pkg);
                config.pkg.insert(pkg.to_owned(), version.clone());
                if is_new {
                    println!("{pkg} {version}");
                }
            }
            None => {
                eprintln!("Error: package {pkg} is not installed");
            }
        }
    }

    fs::write(CONFIG_PATH, toml::to_string_pretty(&config)?)?;
    Ok(())
}

fn installed_package_version(pkg: &str) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let output = ProcessCommand::new("dpkg-query")
        .args(["--show", "--showformat=${Status}\t${Version}\n", pkg])
        .output()?;

    if !output.status.success() {
        if output.status.code() == Some(1) {
            return Ok(None);
        }
        return Err(format!("dpkg-query failed for {pkg}").into());
    }

    let output = String::from_utf8(output.stdout)?;
    let Some((status, version)) = output.trim().split_once('\t') else {
        return Ok(None);
    };
    if status == "install ok installed" {
        Ok(Some(version.to_owned()))
    } else {
        Ok(None)
    }
}

fn install_package(pkg: &str, version: &str) -> Result<(), Box<dyn std::error::Error>> {
    let config = read_config()?;
    fs::create_dir_all(PACKAGE_DIR)?;

    let filename = format!("{pkg}_{version}.deb");
    let package_path = PathBuf::from(PACKAGE_DIR).join(&filename);
    let remote_path = format!(
        "{}/{filename}",
        config.ftp.remote_path.trim_end_matches('/')
    );
    let url = format!(
        "ftp://{}:{}{}",
        config.ftp.host, config.ftp.port, remote_path
    );

    println!("Downloading {filename} from {url}...");
    let status = ProcessCommand::new("wget")
        .args(["--passive-ftp"])
        .args(["--user", &config.ftp.username])
        .args(["--password", &config.ftp.password])
        .args(["--output-document"])
        .arg(&package_path)
        .arg(url)
        .status()?;
    if !status.success() {
        return Err(format!("failed to download {filename}").into());
    }

    let status = ProcessCommand::new("dpkg")
        .arg("--install")
        .arg(&package_path)
        .status()?;
    if !status.success() {
        return Err("dpkg installation failed".into());
    }

    Ok(())
}

fn prompt(label: &str, default: &str) -> io::Result<String> {
    print!("{label} [{default}]: ");
    io::stdout().flush()?;

    let mut value = String::new();
    io::stdin().read_line(&mut value)?;
    let value = value.trim();

    Ok(if value.is_empty() {
        default.to_owned()
    } else {
        value.to_owned()
    })
}

fn initialize_config() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = if PathBuf::from(CONFIG_PATH).exists() {
        read_config()?
    } else {
        Config::default()
    };

    println!("Configure {CONFIG_PATH}. Press Enter to keep the current value.");
    config.normal.version = prompt("normal.version", &config.normal.version)?;
    config.ftp.host = prompt("ftp.host", &config.ftp.host)?;

    let port = prompt("ftp.port", &config.ftp.port.to_string())?;
    config.ftp.port = port.parse()?;
    config.ftp.username = prompt("ftp.username", &config.ftp.username)?;
    config.ftp.password = prompt("ftp.password", &config.ftp.password)?;
    config.ftp.remote_path = prompt("ftp.remote_path", &config.ftp.remote_path)?;

    fs::write(CONFIG_PATH, toml::to_string_pretty(&config)?)?;
    println!("Configuration saved to {CONFIG_PATH}");

    Ok(())
}

fn show_history(pkg: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut log_files: Vec<PathBuf> = fs::read_dir("/var/log")?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("dpkg.log"))
        })
        .collect();
    log_files.sort();
    let mut history = Vec::new();

    for log_file in log_files {
        let output = ProcessCommand::new("zgrep")
            .args(["--fixed-strings", "--no-filename", "--", pkg])
            .arg(&log_file)
            .output()?;

        if output.status.success() {
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                let fields: Vec<&str> = line.split_whitespace().collect();
                let Some(action) = fields.get(2) else {
                    continue;
                };
                let Some(log_pkg) = fields.get(3) else {
                    continue;
                };
                let package_matches = *log_pkg == pkg || log_pkg.starts_with(&format!("{pkg}:"));
                let action_matches = matches!(*action, "upgrade" | "install" | "remove" | "purge");

                if package_matches && action_matches {
                    history.push(line.to_owned());
                }
            }
        } else if output.status.code() != Some(1) {
            return Err(format!("zgrep failed for {}", log_file.display()).into());
        }
    }

    history.sort();
    for line in history {
        println!("{line}");
    }

    Ok(())
}
