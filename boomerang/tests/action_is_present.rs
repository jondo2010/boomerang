//! Tests is_present

use boomerang::prelude::*;

#[reactor]
fn ActionIsPresent(#[state] success: bool) -> impl Reactor {
    let a = builder.add_logical_action::<()>("a", None)?;

    builder
        .add_reaction(None)
        .with_startup_trigger()
        .with_trigger(a)
        .with_reaction_fn(|ctx, state, (_startup, mut a)| {
            if !a.is_present(ctx) {
                assert!(!state.success, "Unexpected");
                println!("Hello startup!");
                ctx.schedule_action(&mut a, (), Some(Duration::nanoseconds(1)));
            } else {
                println!("Hello a!");
                state.success = true;
            }
        })
        .finish()?;

    builder
        .add_reaction(None)
        .with_shutdown_trigger()
        .with_reaction_fn(|_ctx, state, _shutdownn| {
            assert!(state.success, "Failed to print 'Hello World!'");
        })
        .finish()?;
}

#[test]
fn action_is_present() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default().with_fast_forward(true);
    let (_, envs) = boomerang_util::runner::build_and_test_reactor(
        ActionIsPresent(),
        "action_is_present",
        Default::default(),
        config,
    )
    .unwrap();

    let state = envs[0]
        .find_reactor_by_name("action_is_present")
        .and_then(|reactor| reactor.get_state::<ActionIsPresentState>())
        .unwrap();
    assert!(
        state.success,
        "ReactionStartup did not trigger successfully"
    );
}
