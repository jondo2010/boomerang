//! Ported from LF test: modal_models/MixedReactions.lf

use boomerang::prelude::*;

#[reactor]
fn MixedReactions(#[state] x: i32, #[state] first: bool) -> impl Reactor {
    timer! { t(0 s, 100 msec) };

    reaction! {
        (t) {
            state.x = state.x * 10 + 1;
        }
    }

    reaction! {
        (t) {
            state.x = state.x * 10 + 2;
        }
    }

    let mode_a = builder.add_mode("A", true)?;
    let mode_b = builder.add_mode("B", false)?;

    reaction! {
        (t) @modes(mode_a) {
            state.x = state.x * 10 + 3;
            ctx.set_mode_name("B");
        }
    }

    reaction! {
        (t) {
            state.x = state.x * 10 + 4;
        }
    }

    reaction! {
        (t) @modes(mode_b) {
            state.x = state.x * 10 + 5;
        }
    }

    reaction! {
        (t) {
            if state.first {
                assert_eq!(state.x, 1234, "Wrong reaction order on first tick");
                state.first = false;
            } else {
                assert_eq!(state.x, 12341245, "Wrong reaction order on second tick");
                ctx.schedule_shutdown(None);
            }
        }
    }
}

#[test]
fn mixed_reactions() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default()
        .with_fast_forward(true)
        .with_timeout(Duration::milliseconds(250));
    boomerang_util::runner::build_and_test_reactor(
        MixedReactions(),
        "mixed_reactions",
        MixedReactionsState { x: 0, first: true },
        config,
    )
    .unwrap();
}
