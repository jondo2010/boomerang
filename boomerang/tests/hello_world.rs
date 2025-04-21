use boomerang::prelude::*;

#[reactor]
fn HelloWorld2(#[state] success: bool) -> impl Reactor2 {
    builder
        .add_reaction2(None)
        .with_startup_trigger()
        .with_reaction_fn(|_ctx, state, (startup)| {
            println!("Hello World.");
            state.success = true;
        })
        .finish()?;

    builder
        .add_reaction2(None)
        .with_shutdown_trigger()
        .with_reaction_fn(|_ctx, state, (shutdown)| {
            println!("Shutdown invoked.");
            state.success = false;
        })
        .finish()?;
}

#[reactor]
fn HelloWorld() -> impl Reactor2 {
    builder.add_child_reactor2(HelloWorld2(), "_a", Default::default(), false)?;
}

#[test]
fn hello_world() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default().with_fast_forward(true);
    let _ =
        boomerang_util::runner::build_and_test_reactor2(HelloWorld(), "hello_world", (), config)
            .unwrap();
}
