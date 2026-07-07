use boomerang::prelude::*;

#[reactor]
fn ModalActionHistory(
    #[state] scheduled: bool,
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
        let work = builder.add_logical_action::<()>("work", None)?;

        reaction! {
            (pulse) -> work, inactive {
                if !state.scheduled {
                    state.scheduled = true;
                    ctx.schedule_action(&mut work, (), Some(Duration::nanoseconds(5)));
                    inactive.set(ctx);
                }
            }
        }

        reaction! {
            (work) {
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
            assert!(state.fired, "history transition should resume pending action");
            assert_eq!(
                state.fire_time_ns, 7,
                "pending action should fire after its remaining mode-local delay"
            );
        }
    }
}

#[reactor]
fn ModalActionResetDiscard(#[state] scheduled: bool, #[state] fired: bool) -> impl Reactor {
    let pulse = builder.add_logical_action::<()>("pulse", None)?;

    reaction! {
        (startup) -> pulse {
            ctx.schedule_action(&mut pulse, (), Some(Duration::nanoseconds(1)));
            ctx.schedule_action(&mut pulse, (), Some(Duration::nanoseconds(2)));
            ctx.schedule_shutdown(Some(Duration::nanoseconds(10)));
        }
    }

    mode! { initial active {
        let work = builder.add_logical_action::<()>("work", None)?;

        reaction! {
            (pulse) -> work, inactive {
                if !state.scheduled {
                    state.scheduled = true;
                    ctx.schedule_action(&mut work, (), Some(Duration::nanoseconds(5)));
                    inactive.set(ctx);
                }
            }
        }

        reaction! {
            (work) {
                state.fired = true;
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
            assert!(
                !state.fired,
                "reset transition should discard the stale pending action"
            );
        }
    }
}

#[test]
fn modal_action_history_resumes_pending_action() {
    let _ = tracing_subscriber::fmt::try_init();
    let config = runtime::Config::default().with_fast_forward(true);
    boomerang_util::runner::build_and_test_reactor(
        ModalActionHistory(),
        "modal_action_history",
        ModalActionHistoryState {
            scheduled: false,
            fired: false,
            fire_time_ns: 0,
        },
        config,
    )
    .unwrap();
}

#[test]
fn modal_action_reset_discards_pending_action() {
    let _ = tracing_subscriber::fmt::try_init();
    let config = runtime::Config::default().with_fast_forward(true);
    boomerang_util::runner::build_and_test_reactor(
        ModalActionResetDiscard(),
        "modal_action_reset_discard",
        ModalActionResetDiscardState {
            scheduled: false,
            fired: false,
        },
        config,
    )
    .unwrap();
}
