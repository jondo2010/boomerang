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
    let listener = federated::rti::create_listener(12345).await.unwrap();
    let server_handle = tokio::spawn(federated::rti::start_rti(
        listener,
        federated::rti::Config::new(&federation_id).with_federates(federates.len()),
    ));

    let sched_futs = federates
        .into_iter()
        .map(|(federate_key, (env, federate_env))| {
            tracing::info!(?federate_key, %env, "Starting federate.");
            let client_config = federated::client::Config::new(
                federate_key,
                &federation_id,
                federate_env.neighbors.clone(),
            );
            let config =
                runtime::Config::new_federated("127.0.0.1:12345".parse().unwrap(), client_config)
                    .with_fast_forward(fast_forward)
                    .with_keep_alive(keep_alive);
            runtime::Scheduler::new(env, federate_env, config)
        });

    let schedulers = futures::future::try_join_all(sched_futs).await.unwrap();

    let x = schedulers.into_iter().map(|mut sched| {
        tokio::spawn(async move {
            sched.startup().await;
            //scheduler.run().await.unwrap();
        })
    });

    futures::future::try_join_all(x).await.unwrap();

    let handles = server_handle.await.unwrap().unwrap();

    // All federates have connected, and the start-time has been negotiated.

    Ok(())
}
