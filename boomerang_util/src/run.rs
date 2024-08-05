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
}

/// Utility method to build and run a given top-level `Reactor` from tests.
pub fn build_and_test_reactor<R: Reactor>(
    name: &str,
    state: R::State,
    fast_forward: bool,
    keep_alive: bool,
) -> anyhow::Result<R> {
    let mut env_builder = EnvBuilder::new();
    let reactor = R::build(name, state, None, &mut env_builder)
        .context("Error building top-level reactor!")?;
    let env = env_builder.try_into().expect("Error building environment!");
    let mut sched = runtime::Scheduler::new(env, fast_forward, keep_alive);
    sched.event_loop();
    Ok(reactor)
}

/// Utility method to build and run a given top-level `Reactor`. Common arguments are parsed from
/// the command line.
pub fn build_and_run_reactor<R: Reactor>(name: &str, state: R::State) -> anyhow::Result<R> {
    // build the reactor
    let mut env_builder = EnvBuilder::new();
    let reactor = R::build(name, state, None, &mut env_builder)
        .context("Error building top-level reactor!")?;

    {
        let file = std::fs::File::create("recording.json").unwrap();
        let writer = std::io::BufWriter::new(file);
        let serializer = serde_json::Serializer::new(writer);

        let reactor_key = env_builder.find_reactor_by_fqn(name)?;
        let recorder_state =
            crate::recrep::Recorder::new(name, ["snake::keyboard::key_press"], serializer)?;
        let _recorder_builder = crate::recrep::RecorderBuilder::build(
            "recorder",
            recorder_state,
            Some(reactor_key),
            &mut env_builder,
        )?;
    }

    let args = Args::parse();

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
    }

    if args.print_debug_info {
        // runtime::util::print_debug_info(&env);
        println!("{env_builder:#?}");
        panic!("Debug info printed");
    }
    let env = env_builder.try_into().unwrap();

    let mut sched = runtime::Scheduler::new(env, false, true);
    sched.event_loop();

    Ok(reactor)
}
