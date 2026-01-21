use boomerang::prelude::*;

#[reactor]
fn Main(#[state] success: bool) -> impl Reactor {
    let act = builder.add_physical_action::<u32>("act", None)?;
    builder
        .add_reaction(Some("Startup"))
        .with_startup_trigger()
        .with_effect(act)
        .with_reaction_fn(|ctx, _state, (_startup, act)| {
            let send_ctx = ctx.make_send_context();
            let act = act.to_async();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(20));
                send_ctx.schedule_action_async(&act, 434, None);
            });
        })
        .finish()?;

    builder
        .add_reaction(Some("Act"))
        .with_trigger(act)
        .with_reaction_fn(|ctx, mut _state, (mut act,)| {
            let value = ctx.get_action_value(&mut act).unwrap();
            println!("---- Vu {} à {}", value, ctx.get_tag());

            let elapsed_time = ctx.get_elapsed_logical_time();
            assert!(elapsed_time >= Duration::milliseconds(20));
            println!("success");
            _state.success = true;
            ctx.schedule_shutdown(None);
        })
        .finish()?;
}

#[test]
fn physical_action_with_keepalive() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default()
        .with_fast_forward(true)
        .with_keep_alive(true);
    let (_, envs) = boomerang_util::runner::build_and_test_reactor(
        Main(),
        "physical_action_with_keepalive",
        Default::default(),
        config,
    )
    .unwrap();

    let reactor = envs[0]
        .find_reactor_by_name("physical_action_with_keepalive")
        .unwrap();

    let state = reactor.get_state::<MainState>().unwrap();
    assert!(state.success);
}
