use std::path::PathBuf;

use anyhow::{anyhow, Result};
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
    },
}

fn main() -> Result<()> {
    match CargoCli::parse().command {
        CargoCommand::Boomerang(BoomerangArgs {
            workspace,
            command: BoomerangCommand::Build { deployment },
        }) => {
            let manifest = cargo_boomerang::build(workspace, &deployment)?;
            println!("{}", manifest.display());
        }
        CargoCommand::Boomerang(BoomerangArgs {
            workspace,
            command: BoomerangCommand::Check { deployment },
        }) => {
            cargo_boomerang::check(workspace, &deployment)?;
        }
        CargoCommand::Boomerang(BoomerangArgs {
            workspace,
            command: BoomerangCommand::Run { deployment },
        }) => {
            let outcome = cargo_boomerang::run(workspace, &deployment)?;
            std::process::exit(numeric_exit_code(outcome.status())?);
        }
    }
    Ok(())
}

fn numeric_exit_code(status: &std::process::ExitStatus) -> Result<i32> {
    status
        .code()
        .ok_or_else(|| anyhow!("generated application terminated without a numeric exit code"))
}

#[cfg(all(test, unix))]
mod tests {
    use super::numeric_exit_code;
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
}
