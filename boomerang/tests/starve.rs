use boomerang::prelude::*;

#[reactor]
fn Main() -> impl Reactor {
    timer! { t(1 s) };

    let after_shutdown = builder.add_logical_action::<()>("after_shutdown", None)?;

    reaction! {
        (t) -> after_shutdown {
            println!("Timer triggered at {}", ctx.get_tag());
        }
    }

    reaction! {
        (shutdown) -> after_shutdown {
            println!("Shutdown triggered at {}", ctx.get_tag());
            // Ensure that the shutdown is invoked at the correct tag
            assert_eq!(ctx.get_tag(), Tag::new(Duration::seconds(1), 1), "Shutdown invoked at wrong tag");
            ctx.schedule_action(&mut after_shutdown, (), None);
        }
    }

    reaction! {
        (after_shutdown) {
            panic!("Executed a reaction after shutdown");
        }
    }
}

#[test]
fn starve() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default().with_fast_forward(true);
    let (_, _env) =
        boomerang_util::runner::build_and_test_reactor(Main(), "starve", (), config).unwrap();
}
