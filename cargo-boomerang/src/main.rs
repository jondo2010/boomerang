use std::path::PathBuf;

use anyhow::Result;
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
    /// Statically analyze a Boomerang deployment.
    Boomerang(BoomerangArgs),
}

/// Arguments accepted after `cargo boomerang`.
#[derive(Args)]
struct BoomerangArgs {
    /// Application workspace containing `Boomerang.toml` and `Cargo.toml`.
    #[arg(long, default_value = ".", global = true)]
    workspace: PathBuf,
    /// Deployment-tool operation.
    #[command(subcommand)]
    command: BoomerangCommand,
}

/// Deployment-tool operations.
#[derive(Subcommand)]
enum BoomerangCommand {
    /// Validate and lower one named deployment without compiling payload facets.
    Check {
        /// Deployment name declared in `Boomerang.toml`.
        #[arg(long)]
        deployment: String,
    },
}

fn main() -> Result<()> {
    match CargoCli::parse().command {
        CargoCommand::Boomerang(BoomerangArgs {
            workspace,
            command: BoomerangCommand::Check { deployment },
        }) => {
            cargo_boomerang::check(workspace, &deployment)?;
        }
    }
    Ok(())
}
