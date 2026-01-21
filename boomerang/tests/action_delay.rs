//! Test logical action with delay.

use boomerang::prelude::*;

#[reactor]
fn GeneratedDelay(
    #[state] y_state: u32,
    #[input] y_in: u32,
    #[output] y_out: u32,
) -> impl Reactor {
    let act = builder.add_logical_action::<()>("act", Some(Duration::milliseconds(100)))?;

    reaction! {
        YIn (y_in) -> act {
            state.y_state = y_in.unwrap();
            ctx.schedule_action(&mut act, (), None);
        }
    };

    reaction! {
        Act (act) -> y_out {
            *y_out = Some(state.y_state);
        }
    };
}

#[reactor]
fn Source(#[output] out: u32) -> impl Reactor {
    reaction! {
        Startup (startup) -> out {
            *out = Some(1);
        }
    }
}

#[reactor]
fn Sink(#[state] success: bool, #[input] inp: u32) -> impl Reactor {
    reaction! {
        SinkReactionInt (inp) {
            let elapsed_logical = ctx.get_elapsed_logical_time();
            let logical = ctx.get_logical_time();
            let physical = ctx.get_physical_time();
            println!("logical time: {logical:?}");
            println!("physical time: {physical:?}");
            println!("elapsed logical time: {elapsed_logical:?}");
            assert!(
                elapsed_logical == Duration::milliseconds(100),
                "ERROR: Expected 100 msecs but got {elapsed_logical:?}"
            );
            println!("SUCCESS. Elapsed logical time is 100 msec.");
            state.success = true;
        }
    }
}

#[reactor]
fn ActionDelay() -> impl Reactor {
    let source = builder.add_child_reactor(Source(), "source", (), false)?;
    let sink = builder.add_child_reactor(Sink(), "sink", Default::default(), false)?;
    let g = builder.add_child_reactor(GeneratedDelay(), "g", Default::default(), false)?;

    builder.connect_port(source.out, g.y_in, None, false)?;
    builder.connect_port(g.y_out, sink.inp, None, false)?;
}

#[test]
fn action_delay() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default().with_fast_forward(true);
    let (_, env) =
        boomerang_util::runner::build_and_test_reactor(ActionDelay(), "action_delay", (), config)
            .unwrap();

    let sink = env[0]
        .find_reactor_by_name("action_delay/sink")
        .expect("Sink not found");
    let sink_state = sink.get_state::<SinkState>().expect("Sink state not found");
    assert!(sink_state.success, "SinkReactionIn did not trigger");
}
