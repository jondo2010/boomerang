use boomerang::prelude::*;

#[reactor]
fn ModalStartupShutdown(
    #[state] active_startups: u32,
    #[state] active_shutdowns: u32,
    #[state] unreachable_startups: u32,
    #[state] unreachable_shutdowns: u32,
    #[state] startup_microstep: usize,
) -> impl Reactor {
    let enter = builder.add_logical_action::<()>("enter", None)?;
    let leave = builder.add_logical_action::<()>("leave", None)?;
    let enter_again = builder.add_logical_action::<()>("enter_again", None)?;
    let done = builder.add_logical_action::<()>("done", None)?;

    reaction! {
        (startup) -> enter {
            ctx.schedule_action(&mut enter, (), Some(Duration::nanoseconds(1)));
        }
    }

    mode! { initial idle {
        reaction! {
            (enter) -> active {
                active.set(ctx);
            }
        }

        reaction! {
            (enter_again) -> active, done {
                active.set(ctx);
                ctx.schedule_action(&mut done, (), Some(Duration::nanoseconds(1)));
            }
        }
    } }

    mode! { active {
        reaction! {
            (startup) -> leave {
                assert!(startup.is_present(ctx), "startup action should be present");
                state.active_startups += 1;
                state.startup_microstep = ctx.get_microstep();
                ctx.schedule_action(&mut leave, (), Some(Duration::nanoseconds(1)));
            }
        }

        reaction! {
            (leave) -> idle, enter_again {
                idle.set(ctx);
                ctx.schedule_action(&mut enter_again, (), Some(Duration::nanoseconds(1)));
            }
        }

        reaction! {
            (done) -> idle {
                idle.set(ctx);
                ctx.schedule_shutdown(None);
            }
        }

        reaction! {
            (shutdown) {
                assert!(shutdown.is_present(ctx), "shutdown action should be present");
                state.active_shutdowns += 1;
            }
        }
    } }

    mode! { unreachable {
        reaction! {
            (startup) {
                state.unreachable_startups += 1;
            }
        }

        reaction! {
            (shutdown) {
                state.unreachable_shutdowns += 1;
            }
        }
    } }
}

#[reactor]
fn ModalTimeoutShutdown(
    #[state] active_startups: u32,
    #[state] active_shutdowns: u32,
) -> impl Reactor {
    let enter = builder.add_logical_action::<()>("enter", None)?;
    let leave = builder.add_logical_action::<()>("leave", None)?;

    reaction! {
        (startup) -> enter {
            ctx.schedule_action(&mut enter, (), Some(Duration::nanoseconds(1)));
        }
    }

    mode! { initial idle {
        reaction! {
            (enter) -> active {
                active.set(ctx);
            }
        }
    } }

    mode! { active {
        reaction! {
            (startup) -> leave {
                state.active_startups += 1;
                ctx.schedule_action(&mut leave, (), Some(Duration::nanoseconds(1)));
            }
        }

        reaction! {
            (leave) -> idle {
                idle.set(ctx);
            }
        }

        reaction! {
            (shutdown) {
                state.active_shutdowns += 1;
            }
        }
    } }
}

#[test]
fn modal_startup_runs_once_and_shutdown_runs_after_activation() {
    let _ = tracing_subscriber::fmt::try_init();
    let config = runtime::Config::default().with_fast_forward(true);
    let (_, envs) = boomerang_util::runner::build_and_test_reactor(
        ModalStartupShutdown(),
        "modal_startup_shutdown",
        ModalStartupShutdownState {
            active_startups: 0,
            active_shutdowns: 0,
            unreachable_startups: 0,
            unreachable_shutdowns: 0,
            startup_microstep: 0,
        },
        config,
    )
    .unwrap();

    let state = envs[0]
        .find_reactor_by_name("modal_startup_shutdown")
        .and_then(|reactor| reactor.get_state::<ModalStartupShutdownState>())
        .expect("top-level state");

    assert_eq!(
        state.active_startups, 1,
        "mode startup should run on first activation only"
    );
    assert_eq!(
        state.startup_microstep, 1,
        "mode startup should run at the next microstep after activation"
    );
    assert_eq!(
        state.active_shutdowns, 1,
        "mode shutdown should run if the mode was activated, even if inactive at shutdown"
    );
    assert_eq!(
        state.unreachable_startups, 0,
        "unreachable mode startup should not run"
    );
    assert_eq!(
        state.unreachable_shutdowns, 0,
        "unreachable mode shutdown should not run"
    );
}

#[test]
fn modal_timeout_shutdown_uses_activation_history_at_shutdown_time() {
    let _ = tracing_subscriber::fmt::try_init();
    let config = runtime::Config::default()
        .with_fast_forward(true)
        .with_timeout(Duration::nanoseconds(5));
    let (_, envs) = boomerang_util::runner::build_and_test_reactor(
        ModalTimeoutShutdown(),
        "modal_timeout_shutdown",
        ModalTimeoutShutdownState {
            active_startups: 0,
            active_shutdowns: 0,
        },
        config,
    )
    .unwrap();

    let state = envs[0]
        .find_reactor_by_name("modal_timeout_shutdown")
        .and_then(|reactor| reactor.get_state::<ModalTimeoutShutdownState>())
        .expect("top-level state");

    assert_eq!(state.active_startups, 1);
    assert_eq!(
        state.active_shutdowns, 1,
        "timeout shutdown should include modes activated after the timeout was scheduled"
    );
}
