//! Utility methods for building and running reactors from the command line or tests.
//!
//! ## Example:
//!
//! ```rust,ignore
//! fn main() {
//!     let _ =
//!         boomerang_util::runner::build_and_run_reactor::<MyReactor>("my_reactor_instance", ()).unwrap();
//! }
//! ```

use anyhow::Context;
use boomerang::{
    builder::{BuilderRuntimeParts, EnvBuilder, Reactor},
    runtime,
};
use clap::Parser;

#[derive(clap::Parser)]
struct Args {
    /// Generate a graphviz graph of the entire reactor hierarchy
    #[arg(long)]
    full_graph: bool,

    #[arg(long)]
    reaction_graph: bool,

    #[arg(long)]
    print_debug_info: bool,

    #[arg(long, short)]
    fast_forward: bool,

    /// The filename to serialize recorded actions into
    #[cfg(feature = "replay")]
    #[arg(long, value_hint = clap::ValueHint::FilePath)]
    record_filename: Option<std::path::PathBuf>,

    /// The list of fully-qualified actions to record, e.g., "snake::keyboard::key_press"
    #[cfg(feature = "replay")]
    #[arg(long)]
    record_actions: Vec<String>,
}

/// Utility method to build and run a given top-level `Reactor` from tests.
///
/// This method is intended to be used from test functions to build and run a reactor instance.
///
/// # Arguments
///
/// * `name` - The name of the top-level reactor instance
/// * `state` - The initial state of the top-level reactor -- this must match the associated `State` type of the
///     reactor ([`Reactor::State`])
/// * `config` - The runtime configuration for the reactor
///
/// # Environment Variables
///
/// * `PUML` - If this environment variable is set, a PlantUML graph of the reactor hierarchy will be generated
///   and saved to a file named `{name}.puml`.
///
/// # Errors
///
/// This function will return an error if the reactor or environment cannot be built, or if there is an issue
/// writing the PlantUML graph to a file.
///
/// # Returns
///
/// This function returns a tuple containing the built reactor and a vector of runtime environments.
pub fn build_and_test_reactor<R: Reactor>(
    name: &str,
    state: R::State,
    config: runtime::Config,
) -> anyhow::Result<(R, Vec<runtime::Env>)> {
    let mut env_builder = EnvBuilder::new();
    let reactor = R::build(name, state, None, None, false, &mut env_builder)
        .context("Error building top-level reactor!")?;

    if std::env::var("PUML").is_ok() {
        let gv = env_builder.create_plantuml_graph()?;
        let path = format!("{name}.puml");
        let mut f = std::fs::File::create(&path)?;
        std::io::Write::write_all(&mut f, gv.as_bytes())?;
        tracing::info!("Wrote plantuml graph to {path}");
    }

    let BuilderRuntimeParts {
        enclaves,
        aliases: _,
        replayers: _,
    } = env_builder
        .into_runtime_parts()
        .context("Error building environment!")?;

    let envs_out = runtime::execute_enclaves(enclaves.into_iter(), config);
    let envs_out = envs_out.into_iter().map(|(_, env)| env).collect();
    Ok((reactor, envs_out))
}

/// Utility method to build and run a given top-level `Reactor`.
///
/// This method is intended to be used from the `main` function of a binary.
///
/// # Arguments
///
/// * `name` - The name of the top-level reactor instance
/// * `state` - The initial state of the top-level reactor -- this must match the associated `State` type of the
///     reactor ([`Reactor::State`])
///
/// Common arguments are parsed from the command line and passed to the scheduler:
/// * `--full-graph`: Generate a graphviz graph of the entire reactor hierarchy
/// * `--reaction-graph`: Generate a graphviz graph of the reaction hierarchy
/// * `--print-debug-info`: Print debug information about the environment and triggers
/// * `--fast-forward`: Run the scheduler in fast-forward mode
/// * `--record-filename`: The filename to serialize recorded actions into
/// * `--record-actions`: The list of fully-qualified actions to record, e.g., "snake::keyboard::key_press"
pub fn build_and_run_reactor<R: Reactor>(name: &str, state: R::State) -> anyhow::Result<R> {
    // build the reactor
    let mut env_builder = EnvBuilder::new();
    let reactor = R::build(name, state, None, None, false, &mut env_builder)
        .context("Error building top-level reactor!")?;

    let args = Args::parse();

    #[cfg(feature = "replay")]
    let recording_handle = match &args.record_filename {
        Some(filename) => {
            tracing::info!("Recording actions to {filename:?}");

            let opts = runtime::replay::foxglove::McapWriteOptions::new()
                .library(String::from("boomerang-") + env!("CARGO_PKG_VERSION"));
            runtime::replay::foxglove::McapWriter::with_options(opts)
                .create_new_buffered_file(filename)
                .map(Some)?
        }
        None => None,
    };

    if args.full_graph {
        //let gv = graphviz::create_full_graph(&env_builder).unwrap();
        //let path = format!("{name}.dot");
        //let mut f = std::fs::File::create(&path).unwrap();
        //std::io::Write::write_all(&mut f, gv.as_bytes()).unwrap();
        //tracing::info!("Wrote full graph to {path}");
    }

    if args.reaction_graph {
        //let gv = graphviz::create_reaction_graph(&env_builder).unwrap();
        //let path = format!("{name}_levels.dot");
        //let mut f = std::fs::File::create(&path).unwrap();
        //std::io::Write::write_all(&mut f, gv.as_bytes()).unwrap();
        //tracing::info!("Wrote reaction graph to {path}");

        let gv = env_builder.create_plantuml_graph()?;
        let path = format!("{name}.puml");
        let mut f = std::fs::File::create(&path)?;
        std::io::Write::write_all(&mut f, gv.as_bytes())?;
        tracing::info!("Wrote plantuml graph to {path}");
    }

    if args.print_debug_info {
        println!("{env_builder:#?}");
    }
    let BuilderRuntimeParts {
        enclaves,
        aliases: _,
        replayers: _,
    } = env_builder
        .into_runtime_parts()
        .context("Error building environment!")?;

    if args.print_debug_info {
        println!("{enclaves:#?}");
    }

    let config = runtime::Config {
        fast_forward: args.fast_forward,
        ..Default::default()
    };

    let _envs_out = runtime::execute_enclaves(enclaves.into_iter(), config);

    #[cfg(feature = "replay")]
    if let Some(handle) = recording_handle {
        handle.close()?;
    }

    Ok(reactor)
}
