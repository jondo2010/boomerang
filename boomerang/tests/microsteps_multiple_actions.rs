//! Test that scheduling the same logical action multiple times at the same logical
//! time produces multiple events at increasing microsteps (no overwriting).
//! Based on https://github.com/lf-lang/lingua-franca/blob/master/test/Cpp/src/Microsteps.lf

use boomerang::prelude::*;

#[reactor]
fn Destination(
    #[state] seen: Vec<(char, u32)>,
    #[input] x: u32,
    #[input] y: u32,
) -> impl Reactor {
    reaction! {
        (x, y) {
            let elapsed = ctx.get_elapsed_logical_time();
            assert!(elapsed.is_zero(), "Expected elapsed time to stay at zero, got {elapsed:?}");

            let mut present = None;
            if let Some(v) = *x {
                present = Some(('x', v));
            }
            if let Some(v) = *y {
                assert!(present.is_none(), "Both inputs present in the same microstep");
                present = Some(('y', v));
            }

            let which = present.expect("No inputs were present in this microstep");
            state.seen.push(which);
        }
    }
}

#[reactor]
fn MicrostepsMultipleActions() -> impl Reactor {
    let start = builder.add_timer("start", TimerSpec::STARTUP)?;
    let repeat = builder.add_logical_action::<u32>("repeat", None)?;
    let d = builder.add_child_reactor(Destination(), "d", Default::default(), false)?;

    reaction! {
        (start) -> d.x, repeat {
            *d_x = Some(1);
            ctx.schedule_action(&mut repeat, 2, None);
            ctx.schedule_action(&mut repeat, 3, None);
        }
    }

    reaction! {
        (repeat) -> d.y {
            let value = ctx
                .get_action_value(&mut repeat)
                .copied()
                .expect("Missing action payload");
            *d_y = Some(value);
        }
    }
}

#[test]
fn microsteps_multiple_actions() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default().with_fast_forward(true);
    let (_, envs) = boomerang_util::runner::build_and_test_reactor(
        MicrostepsMultipleActions(),
        "microsteps_multiple_actions",
        (),
        config,
    )
    .unwrap();

    let destination = envs[0]
        .find_reactor_by_name("microsteps_multiple_actions/d")
        .expect("Destination not found");
    let state = destination
        .get_state::<DestinationState>()
        .expect("Destination state not found");

    assert_eq!(state.seen, vec![('x', 1), ('y', 2), ('y', 3)]);
}
