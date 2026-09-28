use std::path::PathBuf;
#[cfg(feature = "monitor")]
use std::{io::IsTerminal, net::SocketAddr, num::NonZeroUsize, time::Duration};

use anyhow::{anyhow, Result};
#[cfg(feature = "monitor")]
use boomerang_monitor::{serve, serve_dashboard, MonitorOptions, ReceiverConfig};
use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};

use cargo_boomerang::{ColorChoice, CommandOutput};

/// Cargo plugin entry point.
#[derive(Parser)]
#[command(name = "cargo", bin_name = "cargo")]
struct CargoCli {
    /// Cargo plugin selected by the first positional argument.
    #[command(subcommand)]
    command: CargoCommand,
}

/// Installed Cargo plugins supported by this binary.
#[derive(Subcommand)]
enum CargoCommand {
    /// Statically analyze a Boomerang deployment.
    Boomerang(BoomerangArgs),
}

/// Arguments accepted after `cargo boomerang`.
#[derive(Args)]
struct BoomerangArgs {
    /// Application workspace containing `Boomerang.toml` and `Cargo.toml`.
    #[arg(long, default_value = ".", global = true)]
    workspace: PathBuf,
    /// Suppress cargo-boomerang progress without suppressing command results.
    #[arg(short, long, global = true, conflicts_with = "verbose")]
    quiet: bool,
    /// Use verbose output for nested Cargo commands.
    #[arg(short, long, global = true, action = ArgAction::Count)]
    verbose: u8,
    /// Control color in progress and nested Cargo diagnostics.
    #[arg(long, global = true, value_enum)]
    color: Option<CliColorChoice>,
    /// Deployment-tool operation.
    #[command(subcommand)]
    command: BoomerangCommand,
}

/// Cargo-compatible color values accepted at the command line.
#[derive(Clone, Copy, Debug, ValueEnum)]
enum CliColorChoice {
    /// Select color automatically from the output destination.
    Auto,
    /// Always emit color.
    Always,
    /// Never emit color.
    Never,
}

impl From<CliColorChoice> for ColorChoice {
    fn from(choice: CliColorChoice) -> Self {
        match choice {
            CliColorChoice::Auto => Self::Auto,
            CliColorChoice::Always => Self::Always,
            CliColorChoice::Never => Self::Never,
        }
    }
}

/// Deployment-tool operations.
#[derive(Subcommand)]
enum BoomerangCommand {
    /// Receive framework telemetry and print the completed monitor snapshot.
    #[cfg(feature = "monitor")]
    Monitor {
        /// Local UDP socket address on which to receive telemetry.
        #[arg(long)]
        listen: SocketAddr,
        /// Print the structured snapshot as JSON.
        #[arg(long)]
        json: bool,
        /// Disable the live dashboard and print the completed diagnostic snapshot.
        #[arg(long, conflicts_with = "json")]
        no_interactive: bool,
        /// Stop after this many accepted records; otherwise listen indefinitely.
        #[arg(long)]
        max_records: Option<NonZeroUsize>,
        /// Fail after waiting this long for the next datagram (for example, 5s).
        #[arg(long, value_parser = parse_idle_timeout)]
        idle_timeout: Option<Duration>,
    },
    /// Build and publish one immutable fingerprinted deployment bundle.
    Build {
        /// Deployment name declared in `Boomerang.toml`.
        #[arg(long)]
        deployment: String,
    },
    /// Validate and lower one named deployment without compiling payload facets.
    Check {
        /// Deployment name declared in `Boomerang.toml`.
        #[arg(long)]
        deployment: String,
    },
    /// Build, validate, and run one native generated monolithic deployment.
    Run {
        /// Deployment name declared in `Boomerang.toml`.
        #[arg(long)]
        deployment: String,
        /// Write the completed execution summary as JSON.
        #[arg(short, long)]
        summary: Option<PathBuf>,
    },
}

fn main() -> Result<()> {
    let CargoCommand::Boomerang(BoomerangArgs {
        workspace,
        quiet,
        verbose,
        color,
        command,
    }) = CargoCli::parse().command;
    let color = color
        .map(ColorChoice::from)
        .unwrap_or_else(ColorChoice::from_cargo_env);
    let output = CommandOutput::new(quiet, verbose, color);

    match command {
        #[cfg(feature = "monitor")]
        BoomerangCommand::Monitor {
            listen,
            json,
            no_interactive,
            max_records,
            idle_timeout,
        } => {
            let options = MonitorOptions {
                listen,
                json,
                max_records,
                idle_timeout,
                receiver: ReceiverConfig::default(),
            };
            match monitor_presentation(
                json,
                no_interactive,
                std::io::stdin().is_terminal(),
                std::io::stdout().is_terminal(),
            ) {
                MonitorPresentation::Json => {
                    let snapshot = serve(&options)?;
                    println!("{}", serde_json::to_string_pretty(&snapshot)?);
                }
                MonitorPresentation::Debug => {
                    let snapshot = serve(&options)?;
                    println!("{snapshot:#?}");
                }
                MonitorPresentation::Dashboard => {
                    serve_dashboard(&options)?;
                }
            }
        }
        BoomerangCommand::Build { deployment } => {
            let manifest = cargo_boomerang::build_with_output(workspace, &deployment, &output)?;
            println!("{}", manifest.display());
        }
        BoomerangCommand::Check { deployment } => {
            cargo_boomerang::check_with_output(workspace, &deployment, &output)?;
        }
        BoomerangCommand::Run {
            deployment,
            summary,
        } => {
            let outcome = cargo_boomerang::run_with_output(workspace, &deployment, &output)?;
            if let (Some(path), Some(summary)) = (summary, outcome.summary()) {
                summary.write_json(path)?;
            }
            std::process::exit(numeric_exit_code(outcome.status())?);
        }
    }
    Ok(())
}

#[cfg(feature = "monitor")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MonitorPresentation {
    Dashboard,
    Debug,
    Json,
}

#[cfg(feature = "monitor")]
fn monitor_presentation(
    json: bool,
    no_interactive: bool,
    stdin_is_terminal: bool,
    stdout_is_terminal: bool,
) -> MonitorPresentation {
    if json {
        MonitorPresentation::Json
    } else if no_interactive || !stdin_is_terminal || !stdout_is_terminal {
        MonitorPresentation::Debug
    } else {
        MonitorPresentation::Dashboard
    }
}

#[cfg(feature = "monitor")]
fn parse_idle_timeout(value: &str) -> Result<Duration, String> {
    let duration = humantime::parse_duration(value).map_err(|error| error.to_string())?;
    if duration.is_zero() {
        return Err("idle timeout must be greater than zero".into());
    }
    Ok(duration)
}

fn numeric_exit_code(status: &std::process::ExitStatus) -> Result<i32> {
    status
        .code()
        .ok_or_else(|| anyhow!("generated application terminated without a numeric exit code"))
}

#[cfg(all(test, unix))]
mod tests {
    use super::numeric_exit_code;
    #[cfg(feature = "monitor")]
    use super::{monitor_presentation, MonitorPresentation};
    use std::{os::unix::process::ExitStatusExt, process::ExitStatus};

    #[test]
    fn terminated_process_without_a_numeric_code_is_a_tool_error() {
        let status = ExitStatus::from_raw(15);

        let error = numeric_exit_code(&status).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("terminated without a numeric exit code"),
            "{error:#}"
        );
    }

    #[cfg(feature = "monitor")]
    #[test]
    fn monitor_output_is_interactive_only_for_an_unredirected_terminal() {
        assert_eq!(
            monitor_presentation(false, false, true, true),
            MonitorPresentation::Dashboard
        );
        assert_eq!(
            monitor_presentation(false, false, true, false),
            MonitorPresentation::Debug
        );
        assert_eq!(
            monitor_presentation(false, true, true, true),
            MonitorPresentation::Debug
        );
        assert_eq!(
            monitor_presentation(true, false, true, true),
            MonitorPresentation::Json
        );
    }
}
