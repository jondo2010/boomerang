//! Telemetry monitor for Boomerang applications.
//!
//! Build application packages with ordinary Cargo commands.
//! Enable the `monitor` feature to receive telemetry with `cargo boomerang monitor`.

#[cfg(feature = "monitor")]
use std::{io::IsTerminal, net::SocketAddr, num::NonZeroUsize, time::Duration};

use anyhow::Result;
#[cfg(feature = "monitor")]
use boomerang_monitor::{serve, serve_dashboard, MonitorOptions, ReceiverConfig};
#[cfg(not(feature = "monitor"))]
use clap::CommandFactory;
use clap::{Args, Parser, Subcommand};

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
    /// Receive telemetry from Boomerang applications.
    ///
    /// Build application packages with ordinary Cargo commands.
    /// Enable the `monitor` Cargo feature to receive telemetry.
    Boomerang(BoomerangArgs),
}

/// Receive telemetry from Boomerang applications.
///
/// Build application packages with ordinary Cargo commands.
/// Enable the `monitor` Cargo feature to receive telemetry.
#[derive(Args)]
struct BoomerangArgs {
    /// Telemetry operation. Enable the `monitor` Cargo feature to receive telemetry.
    #[command(subcommand)]
    command: BoomerangCommand,
}

/// Telemetry operations.
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
    /// Reject commands when monitor support is disabled.
    #[cfg(not(feature = "monitor"))]
    #[command(external_subcommand)]
    Unavailable(Vec<String>),
}

fn main() -> Result<()> {
    let CargoCommand::Boomerang(BoomerangArgs { command }) = CargoCli::parse().command;
    #[cfg(feature = "monitor")]
    {
        run(command)
    }
    #[cfg(not(feature = "monitor"))]
    {
        let BoomerangCommand::Unavailable(arguments) = command;
        CargoCli::command()
            .error(
                clap::error::ErrorKind::InvalidSubcommand,
                format!("unrecognized subcommand '{}'", arguments[0]),
            )
            .exit();
    }
}

#[cfg(feature = "monitor")]
fn run(command: BoomerangCommand) -> Result<()> {
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

#[cfg(all(test, feature = "monitor"))]
mod tests {
    use super::{monitor_presentation, MonitorPresentation};

    #[test]
    fn monitor_output_is_interactive_only_for_an_unredirected_terminal() {
        for (json, no_interactive, stdin, stdout, expected) in [
            (false, false, true, true, MonitorPresentation::Dashboard),
            (false, false, true, false, MonitorPresentation::Debug),
            (false, false, false, true, MonitorPresentation::Debug),
            (false, true, true, true, MonitorPresentation::Debug),
            (true, false, true, true, MonitorPresentation::Json),
        ] {
            assert_eq!(
                monitor_presentation(json, no_interactive, stdin, stdout),
                expected
            );
        }
    }
}
