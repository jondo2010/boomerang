use anyhow::Context;
use boomerang::{
    builder::{graphviz, EnvBuilder, Reactor},
    runtime,
};
use clap::Parser;

#[derive(clap::Parser)]
struct Args {
    /// Generate a graphviz graph of the entire reactor hierarchy
    #[arg(short, long)]
    full_graph: bool,

    #[arg(short, long)]
    reaction_graph: bool,

    #[arg(long)]
    print_debug_info: bool,
}

/// Utility method to build and run a given top-level `Reactor`. Common arguments are parsed from
/// the command line.
pub fn build_and_run_reactor<R: Reactor>(name: &str, state: R::State) -> anyhow::Result<R> {
    // build the reactor
    let mut env_builder = EnvBuilder::new();
    let reactor = R::build(name, state, None, &mut env_builder)
        .context("Error building top-level reactor!")?;

    let args = Args::parse();

    if args.reaction_graph {
        let gv = graphviz::create_full_graph(&env_builder).unwrap();
        let mut f = std::fs::File::create(format!("{}.dot", module_path!())).unwrap();
        std::io::Write::write_all(&mut f, gv.as_bytes()).unwrap();
        tracing::info!("Wrote full graph to {}.dot", module_path!());
    }

    if args.reaction_graph {
        let gv = graphviz::create_reaction_graph(&env_builder).unwrap();
        let mut f = std::fs::File::create(format!("{}_levels.dot", module_path!())).unwrap();
        std::io::Write::write_all(&mut f, gv.as_bytes()).unwrap();
        tracing::info!("Wrote reaction graph to {}_levels.dot", module_path!());
    }

    let (env, dep_info) = env_builder.try_into().unwrap();

    if args.print_debug_info {
        runtime::util::print_debug_info(&env, &dep_info);
    }

    runtime::util::assert_consistency(&env, &dep_info);

    let sched = runtime::Scheduler::new(env, dep_info, true);
    sched.event_loop();

    Ok(reactor)
}
