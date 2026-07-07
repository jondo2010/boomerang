use boomerang::prelude::*;

#[reactor]
fn ModalResetReactions(
    #[state] value: i32,
    #[state] reset_count: u32,
    #[state] reset_microstep: usize,
) -> impl Reactor {
    let pulse = builder.add_logical_action::<()>("pulse", None)?;

    reaction! {
        (startup) -> pulse {
            ctx.schedule_action(&mut pulse, (), Some(Duration::nanoseconds(1)));
            ctx.schedule_shutdown(Some(Duration::nanoseconds(10)));
        }
    }

    mode! { initial idle {
        reaction! {
            (pulse) -> active {
                active.set(ctx);
            }
        }
    } }

    mode! { active {
        reaction! {
            (reset) {
                state.value = 0;
                state.reset_count += 1;
                state.reset_microstep = ctx.get_microstep();
                ctx.schedule_shutdown(None);
            }
        }
    } }

    reaction! {
        (shutdown) {
            assert_eq!(state.value, 0, "reset reaction should restore state");
            assert_eq!(state.reset_count, 1, "reset reaction should run once");
            assert_eq!(
                state.reset_microstep, 1,
                "reset reaction should run at the next microstep after transition"
            );
        }
    }
}

#[reactor]
fn ModalInitialDoesNotReset(#[state] reset_count: u32) -> impl Reactor {
    mode! { initial active {
        reaction! {
            (reset) {
                state.reset_count += 1;
            }
        }
    } }

    reaction! {
        (startup) {
            ctx.schedule_shutdown(None);
        }
    }

    reaction! {
        (shutdown) {
            assert_eq!(
                state.reset_count, 0,
                "initial active mode should not run reset reactions at startup"
            );
        }
    }
}

#[test]
fn modal_reset_reaction_runs_on_reset_entry() {
    let _ = tracing_subscriber::fmt::try_init();
    let config = runtime::Config::default().with_fast_forward(true);
    boomerang_util::runner::build_and_test_reactor(
        ModalResetReactions(),
        "modal_reset_reactions",
        ModalResetReactionsState {
            value: 42,
            reset_count: 0,
            reset_microstep: 0,
        },
        config,
    )
    .unwrap();
}

#[test]
fn modal_initial_mode_does_not_run_reset_reaction() {
    let _ = tracing_subscriber::fmt::try_init();
    let config = runtime::Config::default().with_fast_forward(true);
    boomerang_util::runner::build_and_test_reactor(
        ModalInitialDoesNotReset(),
        "modal_initial_does_not_reset",
        ModalInitialDoesNotResetState { reset_count: 0 },
        config,
    )
    .unwrap();
}
