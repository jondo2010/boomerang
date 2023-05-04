use anyhow::Context;

use crate::{
    builder::{EnvBuilder, Reactor},
    federated, runtime,
};

pub async fn build_and_test_federation<R: Reactor>(
    name: &str,
    state: R::State,
    fast_forward: bool,
    keep_alive: bool,
) -> anyhow::Result<()> {
    let mut env_builder = EnvBuilder::new();
    let (reactor_key, _reactor) = R::build(name, state, None, &mut env_builder)
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
        .map(|(federate_key, (env, federate_env))| {
            let config = federated::client::Config::new(
                federate_key,
                &federation_id,
                federate_env.neighbors.clone(),
            );

            //federate_env.env.reactors
            //federate_env.output_control_trigger;

            tokio::spawn(async move {
                let (client, handles) =
                    federated::client::connect_to_rti("127.0.0.1:12345".parse().unwrap(), config)
                        .await
                        .unwrap();

                let mut sched = runtime::Scheduler::new(
                    env,
                    federate_env,
                    runtime::Config::default()
                        .with_fast_forward(fast_forward)
                        .with_keep_alive(keep_alive),
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
