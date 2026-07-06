use boomerang::prelude::*;

#[reactor]
fn ModalBasic(#[state] a_count: u32, #[state] b_count: u32) -> impl Reactor {
    let mode_a = builder.add_mode("mode_a", true)?;
    let mode_b = builder.add_mode("mode_b", false)?;
    let pulse = builder.add_logical_action::<()>("pulse", None)?;

    reaction! {
        (startup) -> pulse {
            ctx.schedule_action(&mut pulse, (), Some(Duration::nanoseconds(1)));
            ctx.schedule_action(&mut pulse, (), Some(Duration::nanoseconds(2)));
        }
    }

    reaction! {
        (pulse) @modes(mode_a) @transition(mode_b) {
            state.a_count += 1;
        }
    }

    reaction! {
        (pulse) @modes(mode_b) @transition(mode_a) {
            state.b_count += 1;
            if state.b_count == 1 {
                ctx.schedule_shutdown(None);
            }
        }
    }

    reaction! {
        (shutdown) {
            assert_eq!(state.a_count, 1, "mode_a reaction should run once");
            assert_eq!(state.b_count, 1, "mode_b reaction should run once");
        }
    }
}

#[test]
fn modal_basic() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default().with_fast_forward(true);
    boomerang_util::runner::build_and_test_reactor(
        ModalBasic(),
        "modal_basic",
        ModalBasicState {
            a_count: 0,
            b_count: 0,
        },
        config,
    )
    .unwrap();
}
