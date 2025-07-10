//! Test asynchronous callbacks that trigger a physical action.

use boomerang::prelude::*;
use std::thread::JoinHandle;

#[derive(Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
struct State {
    #[cfg_attr(feature = "serde", serde(skip))]
    thread: Option<JoinHandle<()>>,
    expected_time: Duration,
    toggle: bool,
    i: usize,
}

#[reactor(state = State)]
fn AsyncCallback() -> impl Reactor2 {
    let t = builder.add_timer(
        "t",
        TimerSpec::default().with_period(Duration::milliseconds(200)),
    )?;
    let a = builder.add_physical_action::<usize>("a", None)?;

    builder
        .add_reaction(Some("T"))
        .with_trigger(t)
        .with_effect(a)
        .with_reaction_fn(|ctx, state, (_t, a)| {
            // make sure to join the old thread first
            if let Some(thread) = state.thread.take() {
                thread.join().unwrap();
            }

            let send_ctx = ctx.make_send_context();
            let a = a.to_async();

            // start new thread
            state.thread = Some(std::thread::spawn(move || {
                // Simulate time passing before a callback occurs
                std::thread::sleep(std::time::Duration::from_millis(100));
                // Schedule twice. If the action is not physical, these should get consolidated into a single action
                // triggering. If it is, then they cause two separate triggerings with close but not equal time stamps.
                send_ctx.schedule_action_async(&a, 0, None);
                send_ctx.schedule_action_async(&a, 0, None);
            }));
        })
        .finish()?;

    builder.add_reaction(Some("A")).with_trigger(a)
    .with_reaction_fn(|ctx, state, (_a, )|{
        let elapsed_time = ctx.get_elapsed_logical_time();
        state.i += 1;
        eprintln!(
            "Asynchronous callback {}: Assigned logical time greater than start time by {elapsed_time:?}",
            state.i,
        );
        if elapsed_time <= state.expected_time {
            panic!(
                "ERROR: Expected logical time to be larger than {:?}, was {elapsed_time:?}",
                state.expected_time
            );
        }
        if state.toggle {
            state.toggle = false;
            state.expected_time += Duration::milliseconds(200);
        } else {
            state.toggle = true;
        }
    })
    .finish()?;

    builder
        .add_reaction(Some("Shutdown"))
        .with_shutdown_trigger()
        .with_reaction_fn(|_ctx, state, _| {
            // make sure to join the thread before shutting down
            if state.thread.is_some() {
                std::mem::take(&mut state.thread).unwrap().join().unwrap();
            }
        })
        .finish()?;
}

#[test]
fn async_callback() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default()
        .with_fast_forward(false)
        .with_timeout(Duration::seconds(2));
    let _ = boomerang_util::runner::build_and_test_reactor2(
        AsyncCallback(),
        "async_callback",
        State {
            thread: None,
            expected_time: Duration::milliseconds(100),
            toggle: false,
            i: 0,
        },
        config,
    )
    .unwrap();
}
