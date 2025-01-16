#![feature(exit_status_error)]

use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::{Parser, Subcommand};
use colored::Colorize;
use config::Config;

mod config;

const ROOT_FILE_NAME: &str = "dotty.toml";
const DEFAULT_STATE_FILE_NAME: &str = "dotty.state.toml";

/// Dotty - A CLI based dotfile and package manager
#[derive(Parser, Debug)]
struct CliCommand {
    /// Config file
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// State file
    #[arg(short, long)]
    state: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

impl CliCommand {
    fn config_path(&self) -> PathBuf {
        self.config
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(ROOT_FILE_NAME))
    }

    fn state_path(&self) -> PathBuf {
        self.state
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_STATE_FILE_NAME))
    }
}

#[derive(Subcommand, Debug, Clone)]
enum Command {
    /// Debug commands
    #[command(subcommand)]
    Debug(DebugCommand),
    /// Init the config from a default template
    Init {
        /// Path to create config at
        path: Option<PathBuf>,
    },
    /// Apply migrations
    Apply,
    /// Update stuff
    Update,
}

#[derive(Subcommand, Debug, Clone)]
enum DebugCommand {
    /// Prints the current configuration
    PrintConfig,
    /// Prints the current state
    PrintState,
    /// Prints the difference between the current state and the config
    PrintDiff,
    /// Prints the actions to be performed
    PrintActions,
}

fn main() -> Result<()> {
    let cli = CliCommand::parse();
    match cli.command.clone() {
        Command::Debug(debug) => do_debug(cli, debug)?,
        Command::Init { path } => {
            let path = path.unwrap_or_else(|| PathBuf::from(ROOT_FILE_NAME));
            create_default_config(&path)?;
        }
        Command::Apply => {
            let config = read_config(&cli.config_path())?;
            let state = read_config(&cli.state_path()).unwrap_or_default();

            let diff = config.diff(state)?;
            for change in diff {
                println!("[*] {}", change.render());
                let actions = change.action(&config)?;
                for action in actions {
                    println!("[>] {}", action.render());
                    action.execute()?
                }
            }

            write_config(&cli.state_path(), &config)?;
        }
        Command::Update => {
            let config = read_config(&cli.config_path())?;
            let state = read_config(&cli.state_path()).unwrap_or_default();

            let changes = config.update()?;
            for change in changes {
                println!("[*] {}", change.render());
                let actions = change.action(&config)?;
                for action in actions {
                    println!("[>] {}", action.render());
                    action.execute()?
                }
            }
            write_config(&cli.state_path(), &config)?;
        }
    }

    Ok(())
}

fn do_debug(cli: CliCommand, debug: DebugCommand) -> Result<(), anyhow::Error> {
    match debug {
        DebugCommand::PrintConfig => {
            let config = read_config(&cli.config_path())?;
            dbg!(config);
        }
        DebugCommand::PrintState => {
            let state = read_config(&cli.state_path()).unwrap_or_default();
            dbg!(state);
        }
        DebugCommand::PrintDiff => {
            let config = read_config(&cli.config_path())?;
            let state = read_config(&cli.state_path()).unwrap_or_default();
            let diff = config.diff(state)?;
            for change in diff {
                println!("[{}] {}", change.priority(&config), change.render());
            }
        }
        DebugCommand::PrintActions => {
            let config = read_config(&cli.config_path())?;
            let state = read_config(&cli.state_path()).unwrap_or_default();
            let diff = config.diff(state)?;
            for change in diff {
                let actions = change.action(&config)?;
                for action in actions {
                    println!("{}", action.render());
                }
            }
        }
    }
    Ok(())
}

fn read_config(path: &Path) -> Result<Config> {
    println!("Reading config at {}", path.to_string_lossy().blue());

    let content = std::fs::read_to_string(path)?;
    let mut config: Config = toml::from_str(&content)?;
    let directory = path.parent().unwrap_or(Path::new("."));
    config.load_dependencies(directory)?;
    Ok(config)
}

fn write_config(path: &Path, config: &Config) -> Result<()> {
    println!("Writing config at {}", path.to_string_lossy().blue());

    let content = toml::to_string(config)?;
    std::fs::write(path, content)?;

    Ok(())
}

fn create_default_config(path: &Path) -> Result<()> {
    println!("Creating config at {}", path.to_string_lossy().blue());

    let config = Config::example();
    let content = toml::to_string(&config)?;
    std::fs::write(path, content)?;

    Ok(())
}
