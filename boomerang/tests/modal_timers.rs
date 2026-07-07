use boomerang::prelude::*;

#[reactor]
fn ModalTimerHistory(
    #[state] exited: bool,
    #[state] fired: bool,
    #[state] fire_time_ns: i128,
) -> impl Reactor {
    let pulse = builder.add_logical_action::<()>("pulse", None)?;

    reaction! {
        (startup) -> pulse {
            ctx.schedule_action(&mut pulse, (), Some(Duration::nanoseconds(1)));
            ctx.schedule_action(&mut pulse, (), Some(Duration::nanoseconds(2)));
            ctx.schedule_shutdown(Some(Duration::nanoseconds(10)));
        }
    }

    mode! { initial active {
        let tick = builder.add_timer(
            "tick",
            TimerSpec::default().with_offset(Duration::nanoseconds(5)),
        )?;

        reaction! {
            (pulse) -> inactive {
                if !state.exited {
                    state.exited = true;
                    inactive.set(ctx);
                }
            }
        }

        reaction! {
            (tick) {
                state.fired = true;
                state.fire_time_ns = ctx.get_elapsed_logical_time().whole_nanoseconds();
            }
        }
    } }

    mode! { inactive {
        reaction! {
            (pulse) -> history(active) {
                active.set(ctx);
            }
        }
    } }

    reaction! {
        (shutdown) {
            assert!(state.fired, "history transition should resume pending timer");
            assert_eq!(
                state.fire_time_ns, 6,
                "timer should fire after the remaining active local delay"
            );
        }
    }
}

#[reactor]
fn ModalTimerReset(
    #[state] exited: bool,
    #[state] fired_count: u32,
    #[state] fire_time_ns: i128,
) -> impl Reactor {
    let pulse = builder.add_logical_action::<()>("pulse", None)?;

    reaction! {
        (startup) -> pulse {
            ctx.schedule_action(&mut pulse, (), Some(Duration::nanoseconds(1)));
            ctx.schedule_action(&mut pulse, (), Some(Duration::nanoseconds(2)));
            ctx.schedule_shutdown(Some(Duration::nanoseconds(10)));
        }
    }

    mode! { initial active {
        let tick = builder.add_timer(
            "tick",
            TimerSpec::default().with_offset(Duration::nanoseconds(5)),
        )?;

        reaction! {
            (pulse) -> inactive {
                if !state.exited {
                    state.exited = true;
                    inactive.set(ctx);
                }
            }
        }

        reaction! {
            (tick) {
                state.fired_count += 1;
                state.fire_time_ns = ctx.get_elapsed_logical_time().whole_nanoseconds();
            }
        }
    } }

    mode! { inactive {
        reaction! {
            (pulse) -> reset(active) {
                active.set(ctx);
            }
        }
    } }

    reaction! {
        (shutdown) {
            assert_eq!(state.fired_count, 1, "reset should restart the timer once");
            assert_eq!(
                state.fire_time_ns, 7,
                "reset timer should fire from the reset entry time"
            );
        }
    }
}

#[test]
fn modal_timer_history_resumes_pending_timer() {
    let _ = tracing_subscriber::fmt::try_init();
    let config = runtime::Config::default().with_fast_forward(true);
    boomerang_util::runner::build_and_test_reactor(
        ModalTimerHistory(),
        "modal_timer_history",
        ModalTimerHistoryState {
            exited: false,
            fired: false,
            fire_time_ns: 0,
        },
        config,
    )
    .unwrap();
}

#[test]
fn modal_timer_reset_restarts_timer() {
    let _ = tracing_subscriber::fmt::try_init();
    let config = runtime::Config::default().with_fast_forward(true);
    boomerang_util::runner::build_and_test_reactor(
        ModalTimerReset(),
        "modal_timer_reset",
        ModalTimerResetState {
            exited: false,
            fired_count: 0,
            fire_time_ns: 0,
        },
        config,
    )
    .unwrap();
}
