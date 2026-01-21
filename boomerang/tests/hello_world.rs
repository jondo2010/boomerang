use boomerang::prelude::*;

#[reactor]
fn HelloWorld2(#[state] success: bool) -> impl Reactor {
    reaction! {
        (startup) {
            assert!(
                startup.is_present(ctx),
                "The startup action should be present."
            );
            println!("Hello World.");
            state.success = true;
        }
    }

    reaction! {
        (shutdown) {
            assert!(
                shutdown.is_present(ctx),
                "The shutdown action should be present."
            );
            println!("Shutdown invoked.");
            assert!(
                state.success,
                "The startup action should have set the state to true."
            );
            state.success = false;
        }
    }
}

#[reactor]
fn HelloWorld() -> impl Reactor {
    builder.add_child_reactor(HelloWorld2(), "_a", Default::default(), false)?;
}

#[test]
fn hello_world() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default().with_fast_forward(true);
    let _ =
        boomerang_util::runner::build_and_test_reactor(HelloWorld(), "hello_world", (), config)
            .unwrap();
}
