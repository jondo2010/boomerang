//! Test logical action with delay.

use boomerang::prelude::*;

#[reactor]
fn Child(act: TypedActionKey<i32>) -> impl Reactor<(), Ports = ChildPorts> {
    builder
        .add_reaction(None)
        .with_trigger(act)
        .with_reaction_fn(|ctx, _state, (mut act,)| {
            let value = ctx.get_action_value(&mut act);
            println!("[child received: {value:?}]");
        })
        .finish()?;
}

#[reactor]
fn ActionValues(#[state] r1done: bool, #[state] r2done: bool) -> Reactor {
    let act = builder.add_logical_action::<i32>("act", Some(Duration::milliseconds(100)))?;

    let _child = builder.add_child_reactor(Child(act), "child", (), false)?;

    builder
        .add_reaction(Some("Startup"))
        .with_startup_trigger()
        .with_effect(act)
        .with_reaction_fn(|ctx, _state, (_startup, mut act)| {
            println!("Startup reaction");
            ctx.schedule_action(&mut act, 100, None);
            ctx.schedule_action(&mut act, -100, Some(Duration::milliseconds(50)));
        })
        .finish()?;

    builder
        .add_reaction(Some("Act"))
        .with_trigger(act)
        .with_reaction_fn(|ctx, state, (mut act,)| {
            let elapsed = ctx.get_elapsed_logical_time();
            let value = ctx.get_action_value(&mut act);

            println!("[@{elapsed:?} action transmitted: {value:?}]");

            if elapsed.whole_milliseconds() == 100 {
                assert_eq!(value, Some(&100), "ERROR: Expected action value to be 100");
                state.r1done = true;
            } else {
                if elapsed.whole_milliseconds() != 150 {
                    panic!("ERROR: Unexpected reaction invocation at {elapsed:?}");
                }
                assert_eq!(
                    value,
                    Some(&-100),
                    "ERROR: Expected action value to be -100"
                );
                state.r2done = true;
            }
        })
        .finish()?;

    builder
        .add_reaction(Some("Shutdown"))
        .with_shutdown_trigger()
        .with_reaction_fn(|_ctx, state, (_shutdown,)| {
            assert!(
                state.r1done && state.r2done,
                "ERROR: Expected 2 reaction invocations\n"
            );
            println!("Ok.");
        })
        .finish()?;
}

#[test]
fn action_values() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default().with_fast_forward(true);
    let _ = boomerang_util::runner::build_and_test_reactor(
        ActionValues(),
        "action_values",
        ActionValuesState {
            r1done: false,
            r2done: false,
        },
        config,
    )
    .unwrap();
}
