use boomerang::prelude::*;

#[reactor]
fn ModalPhysicalAction(#[state] fired: bool) -> impl Reactor {
    let resume = builder.add_logical_action::<()>("resume", None)?;

    reaction! {
        (startup) -> resume {
            ctx.schedule_action(&mut resume, (), Some(Duration::milliseconds(30)));
            ctx.schedule_shutdown(Some(Duration::milliseconds(70)));
        }
    }

    mode! { initial active {
        let physical = builder.add_physical_action::<()>("physical", None)?;
        let leave = builder.add_logical_action::<()>("leave", None)?;

        reaction! {
            (startup) -> physical, leave {
                ctx.schedule_action(&mut physical, (), Some(Duration::milliseconds(20)));
                ctx.schedule_action(&mut leave, (), Some(Duration::nanoseconds(1)));
            }
        }

        reaction! {
            (leave) -> idle {
                idle.set(ctx);
            }
        }

        reaction! {
            (physical) {
                state.fired = true;
            }
        }
    } }

    mode! { idle {
        reaction! {
            (resume) -> history(active) {
                active.set(ctx);
            }
        }
    } }

    reaction! {
        (shutdown) {
            assert!(
                !state.fired,
                "physical action should not be suspended and replayed by history"
            );
        }
    }
}

#[test]
fn modal_physical_action_is_dropped_when_inactive_at_event_tag() {
    let _ = tracing_subscriber::fmt::try_init();
    let config = runtime::Config::default().with_fast_forward(false);
    let (_, envs) = boomerang_util::runner::build_and_test_reactor(
        ModalPhysicalAction(),
        "modal_physical_action",
        ModalPhysicalActionState { fired: false },
        config,
    )
    .unwrap();

    let state = envs[0]
        .find_reactor_by_name("modal_physical_action")
        .and_then(|reactor| reactor.get_state::<ModalPhysicalActionState>())
        .expect("top-level state");

    assert!(!state.fired);
}
