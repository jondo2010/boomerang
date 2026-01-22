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
use std::path::PathBuf;

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

    /// The filename to replay serialized data from
    #[cfg(feature = "replay")]
    #[arg(long, value_hint = clap::ValueHint::FilePath, conflicts_with = "record_filename")]
    replay_filename: Option<std::path::PathBuf>,
}

fn sanitize_diagram_stem(name: &str) -> String {
    let mut sanitized = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            sanitized.push(ch);
        } else {
            sanitized.push('_');
        }
    }
    if sanitized.is_empty() {
        "reactor".to_string()
    } else {
        sanitized
    }
}

fn diagram_output_path(name: &str, extension: &str) -> anyhow::Result<PathBuf> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .unwrap_or(manifest_dir.as_path())
        .to_path_buf();
    let mut path = workspace_root;
    path.push("target");
    path.push("boomerang");
    path.push("diagrams");
    std::fs::create_dir_all(&path).context("Failed to create diagram output directory")?;
    path.push(format!("{}.{}", sanitize_diagram_stem(name), extension));
    Ok(path)
}

fn env_flag_enabled(var: &str) -> bool {
    let value = match std::env::var(var) {
        Ok(value) => value,
        Err(_) => return false,
    };
    let value = value.trim().to_ascii_lowercase();
    matches!(value.as_str(), "1" | "true" | "yes" | "on")
}

pub fn build_and_test_reactor<S: runtime::ReactorData, R: Reactor<S>>(
    reactor_builder: R,
    name: &str,
    state: S,
    config: runtime::Config,
) -> anyhow::Result<(R::Ports, Vec<runtime::Env>)> {
    let mut env_builder = EnvBuilder::new();
    let reactor = reactor_builder
        .build(name, state, None, None, false, &mut env_builder)
        .context("Error building top-level reactor!")?;

    env_builder.validate_reactions()?;

    if env_flag_enabled("BOOMERANG_REACTION_GRAPH") {
        let gv = env_builder.create_plantuml_graph()?;
        let path = diagram_output_path(name, "puml")?;
        let mut f = std::fs::File::create(&path)?;
        std::io::Write::write_all(&mut f, gv.as_bytes())?;
        tracing::info!("Wrote plantuml graph to {}", path.display());
    }

    let BuilderRuntimeParts {
        enclaves,
        aliases: _,
        ..
    } = env_builder
        .into_runtime_parts(&config)
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
/// * `state` - The initial state of the top-level reactor; this must match the state type the reactor expects.
///
/// Common arguments are parsed from the command line and passed to the scheduler:
/// * `--full-graph`: Generate a graphviz graph of the entire reactor hierarchy
/// * `--reaction-graph`: Generate a graphviz graph of the reaction hierarchy
/// * `--print-debug-info`: Print debug information about the environment and triggers
/// * `--fast-forward`: Run the scheduler in fast-forward mode
/// * `--record-filename`: The filename to serialize recorded actions into
/// * `--record-actions`: The list of fully-qualified actions to record, e.g., "snake::keyboard::key_press"
pub fn build_and_run_reactor<S, R>(reactor: R, name: &str, state: S) -> anyhow::Result<R::Ports>
where
    S: runtime::ReactorData,
    R: Reactor<S>,
{
    // build the reactor
    let mut env_builder = EnvBuilder::new();
    let reactor = reactor
        .build(name, state, None, None, false, &mut env_builder)
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
        let path = diagram_output_path(name, "puml")?;
        let mut f = std::fs::File::create(&path)?;
        std::io::Write::write_all(&mut f, gv.as_bytes())?;
        tracing::info!("Wrote plantuml graph to {}", path.display());
    }

    if args.print_debug_info {
        println!("{env_builder:#?}");
    }

    let config = runtime::Config {
        fast_forward: args.fast_forward,
        ..Default::default()
    };

    let BuilderRuntimeParts {
        enclaves,
        aliases: _,
        #[cfg(feature = "replay")]
        replayers,
    } = env_builder
        .into_runtime_parts(&config)
        .context("Error building environment!")?;

    if args.print_debug_info {
        println!("{enclaves:#?}");
    }

    #[cfg(feature = "replay")]
    if let Some(filename) = args.replay_filename {
        tracing::info!("Reading replay from {}", filename.display());
        runtime::replay::create_replayer(filename, replayers, &enclaves)?;
    }

    let _envs_out = runtime::execute_enclaves(enclaves.into_iter(), config);

    #[cfg(feature = "replay")]
    if let Some(handle) = recording_handle {
        handle.close()?;
    }

    Ok(reactor)
}
