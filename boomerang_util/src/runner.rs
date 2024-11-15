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
    builder::{graphviz, EnvBuilder, Reactor},
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
pub fn build_and_test_reactor<R: Reactor>(
    name: &str,
    state: R::State,
    config: runtime::Config,
) -> anyhow::Result<(R, runtime::Scheduler)> {
    let mut env_builder = EnvBuilder::new();
    let reactor = R::build(name, state, None, None, &mut env_builder)
        .context("Error building top-level reactor!")?;

    if std::env::var("PUML").is_ok() {
        let gv = env_builder.create_plantuml_graph()?;
        let path = format!("{name}.puml");
        let mut f = std::fs::File::create(&path)?;
        std::io::Write::write_all(&mut f, gv.as_bytes())?;
        tracing::info!("Wrote plantuml graph to {path}");
    }

    let (env, graph, _) = env_builder
        .into_runtime_parts()
        .context("Error building environment!")?;
    let mut sched = runtime::Scheduler::new(env, graph, config);
    sched.event_loop();
    Ok((reactor, sched))
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
    let reactor = R::build(name, state, None, None, &mut env_builder)
        .context("Error building top-level reactor!")?;

    let args = Args::parse();

    #[cfg(feature = "replay")]
    if let Some(filename) = args.record_filename {
        tracing::info!("Recording actions to {filename:?}");
        crate::replay::inject_recorder(
            &mut env_builder,
            filename,
            name,
            args.record_actions.iter().map(|s| s.as_str()),
        )?;
    }

    if args.full_graph {
        let gv = graphviz::create_full_graph(&env_builder).unwrap();
        let path = format!("{name}.dot");
        let mut f = std::fs::File::create(&path).unwrap();
        std::io::Write::write_all(&mut f, gv.as_bytes()).unwrap();
        tracing::info!("Wrote full graph to {path}");
    }

    if args.reaction_graph {
        let gv = graphviz::create_reaction_graph(&env_builder).unwrap();
        let path = format!("{name}_levels.dot");
        let mut f = std::fs::File::create(&path).unwrap();
        std::io::Write::write_all(&mut f, gv.as_bytes()).unwrap();
        tracing::info!("Wrote reaction graph to {path}");

        let gv = env_builder.create_plantuml_graph()?;
        let path = format!("{name}.puml");
        let mut f = std::fs::File::create(&path)?;
        std::io::Write::write_all(&mut f, gv.as_bytes())?;
        tracing::info!("Wrote plantuml graph to {path}");
    }

    if args.print_debug_info {
        println!("{env_builder:#?}");
    }
    let (env, triggers, _) = env_builder
        .into_runtime_parts()
        .context("Error building environment!")?;
    if args.print_debug_info {
        println!("{env:#?}");
        println!("{triggers:#?}");
    }

    let config = runtime::Config {
        fast_forward: args.fast_forward,
        ..Default::default()
    };

    let mut sched = runtime::Scheduler::new(env, triggers, config);
    sched.event_loop();

    Ok(reactor)
}
