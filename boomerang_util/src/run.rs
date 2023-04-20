use crate::{
    builder::{graphviz, BuilderReactorKey, EnvBuilder, Reactor},
    runtime,
};
use anyhow::Context;
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
#[cfg(not(feature = "federated"))]
pub fn build_and_test_reactor<R: Reactor>(
    name: &str,
    state: R::State,
    fast_forward: bool,
    keep_alive: bool,
) -> anyhow::Result<(BuilderReactorKey, R)> {
    let mut env_builder = EnvBuilder::new();
    let reactor = R::build(name, state, None, &mut env_builder)
        .context("Error building top-level reactor!")?;
    let env = env_builder
        .build_runtime(None)
        .expect("Error building environment!");
    let mut sched = runtime::Scheduler::new(
        env,
        runtime::Config::default()
            .with_fast_forward(fast_forward)
            .with_keep_alive(keep_alive),
    );
    sched.event_loop();
    Ok(reactor)
}

#[cfg(feature = "federated")]
pub async fn build_and_test_federation<R: Reactor>(
    name: &str,
    state: R::State,
    fast_forward: bool,
    keep_alive: bool,
) -> anyhow::Result<()> {
    use boomerang::federated;

    let mut env_builder = EnvBuilder::new();
    let (reactor_key, reactor) = R::build(name, state, None, &mut env_builder)
        .context("Error building top-level reactor!")?;

    let federates = env_builder
        .federalize_reactor(reactor_key)
        .context("Error federalizing reactor!")?;

    let federation_id = format!("{name}_federation");

    // Spawn the RTI server
    let mut rti = federated::rti::Rti::new(federates.len(), &federation_id);
    let listener = rti.create_listener(12345).await.unwrap();
    let server_handle = tokio::spawn(async move { rti.start_server(listener).await });

    let clients = federates
        .into_iter()
        .map(|(federate_key, (env, neighbors))| {
            let config =
                federated::client::Config::new(federate_key, &federation_id, neighbors.clone());

            tokio::spawn(async move {
                let (client, handles) =
                    federated::client::connect_to_rti("127.0.0.1:12345".parse().unwrap(), config)
                        .await
                        .unwrap();

                let mut sched = runtime::Scheduler::new(
                    env,
                    runtime::Config::default()
                        .with_fast_forward(false)
                        .with_keep_alive(true),
                    handles,
                    client,
                );

                tokio::task::spawn_blocking(move || sched.event_loop());
            })
        })
        .collect::<Vec<_>>();

    let handles = server_handle.await.unwrap().unwrap();

    // All federates have connected, and the start-time has been negotiated.

    for c in clients {
        c.await.unwrap();
    }

    for f in handles.federate_handles {
        f.await.unwrap();
    }

    Ok(())
}

/// Utility method to build and run a given top-level `Reactor`. Common arguments are parsed from
/// the command line.
#[cfg(not(feature = "federated"))]
pub fn build_and_run_reactor<R: Reactor>(
    name: &str,
    state: R::State,
) -> anyhow::Result<(BuilderReactorKey, R)> {
    // build the reactor
    let mut env_builder = EnvBuilder::new();
    let reactor = R::build(name, state, None, &mut env_builder)
        .context("Error building top-level reactor!")?;

    let args = Args::parse();

    if args.full_graph {
        let gv = graphviz::create_full_graph(&env_builder, graphviz::Config::default()).unwrap();
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
    }
    let env = env_builder.build_runtime(None).unwrap();

    let mut sched = runtime::Scheduler::new(
        env,
        runtime::Config::default()
            .with_fast_forward(args.fast_forward)
            .with_keep_alive(true),
    );
    sched.event_loop();

    Ok(reactor)
}
