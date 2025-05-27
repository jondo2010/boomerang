use boomerang::prelude::*;

#[reactor]
fn Main() -> impl Reactor2 {
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
            if ctx.get_elapsed_logical_time() != Duration::seconds(1) || ctx.get_microstep() != 1 {
                eprintln!("Shutdown invoked at wrong tag");
                std::process::exit(2);
            }
            ctx.schedule_action(&mut after_shutdown, (), None);
        }
    }

    reaction! {
        (after_shutdown) {
            eprintln!("Executed a reaction after shutdown");
            std::process::exit(1);
        }
    }
}

#[test]
fn starve() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default().with_fast_forward(true);
    let (_, _env) =
        boomerang_util::runner::build_and_test_reactor2(Main(), "starve", (), config).unwrap();
}
