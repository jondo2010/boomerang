use boomerang::prelude::*;

#[derive(Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
struct Count3State {
    sequence: Vec<String>,
}

#[reactor(state = Count3State)]
fn ModalCount3Modes() -> impl Reactor {
    let pulse = ctx.add_logical_action::<()>("pulse", None)?;
    let advance = ctx.add_logical_action::<()>("advance", None)?;

    reaction! {
        (startup) -> pulse {
            ctx.schedule_action(&mut pulse, (), Some(Duration::nanoseconds(1)));
        }
    }

    reaction! {
        (advance) -> pulse {
            ctx.schedule_action(&mut pulse, (), Some(Duration::nanoseconds(1)));
        }
    }

    mode! { initial one {
        reaction! {
            (pulse) -> two, advance {
                state.sequence.push("one".to_owned());
                two.set(ctx);
                ctx.schedule_action(&mut advance, (), None);
            }
        }
    } }

    mode! { two {
        reaction! {
            (pulse) -> three, advance {
                state.sequence.push("two".to_owned());
                three.set(ctx);
                ctx.schedule_action(&mut advance, (), None);
            }
        }
    } }

    mode! { three {
        reaction! {
            (pulse) -> one, advance {
                state.sequence.push("three".to_owned());
                if state.sequence.len() == 6 {
                    ctx.schedule_shutdown(None);
                } else {
                    one.set(ctx);
                    ctx.schedule_action(&mut advance, (), None);
                }
            }
        }
    } }

    reaction! {
        (shutdown) {
            assert_eq!(
                state.sequence,
                ["one", "two", "three", "one", "two", "three"],
                "only the active mode should respond to each pulse"
            );
        }
    }
}

#[test]
fn modal_count_3_modes_cycles_active_mode_only() {
    let _ = tracing_subscriber::fmt::try_init();
    let config = runtime::Config::default().with_fast_forward(true);
    let (_, envs) = boomerang_util::runner::build_and_test_reactor(
        ModalCount3Modes(),
        "modal_count_3_modes",
        Count3State::default(),
        config,
    )
    .unwrap();

    let state = envs[0]
        .find_reactor_by_name("modal_count_3_modes")
        .and_then(|reactor| reactor.get_state::<Count3State>())
        .expect("top-level state");

    assert_eq!(
        state.sequence,
        ["one", "two", "three", "one", "two", "three"]
    );
}
