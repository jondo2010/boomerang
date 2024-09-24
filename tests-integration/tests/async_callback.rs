//! Test asynchronous callbacks that trigger a physical action.

use boomerang::{builder::prelude::*, runtime, Reaction, Reactor};
use boomerang_util::timeout;
use std::thread::JoinHandle;

struct State {
    thread: Option<JoinHandle<()>>,
    expected_time: runtime::Duration,
    toggle: bool,
    i: usize,
}

#[derive(Clone, Reactor)]
#[reactor(
    state = "State",
    reaction = "ReactionT",
    reaction = "ReactionA",
    reaction = "ReactionShutdown"
)]
struct AsyncCallback {
    #[reactor(timer(period = "200 msec"))]
    t: TimerActionKey,

    a: TypedActionKey<usize, Physical>,

    #[reactor(child = runtime::Duration::from_secs(2))]
    _timeout: timeout::Timeout,
}

#[derive(Reaction)]
#[reaction(reactor = "AsyncCallback", triggers(action = "t"))]
struct ReactionT {
    a: runtime::PhysicalActionRef<usize>,
}

impl Trigger<AsyncCallback> for ReactionT {
    fn trigger(self, ctx: &mut runtime::Context, state: &mut <AsyncCallback as Reactor>::State) {
        // make sure to join the old thread first
        if let Some(thread) = state.thread.take() {
            thread.join().unwrap();
        }

        let mut send_ctx = ctx.make_send_context();
        let mut a = self.a.clone();

        // start new thread
        state.thread = Some(std::thread::spawn(move || {
            // Simulate time passing before a callback occurs
            std::thread::sleep(runtime::Duration::from_millis(100));
            // Schedule twice. If the action is not physical, these should get consolidated into a single action
            // triggering. If it is, then they cause two separate triggerings with close but not equal time stamps.
            send_ctx.schedule_action(&mut a, Some(0), None);
            send_ctx.schedule_action(&mut a, Some(0), None);
        }));
    }
}

#[derive(Reaction)]
#[reaction(reactor = "AsyncCallback", triggers(action = "a"))]
struct ReactionA;

impl Trigger<AsyncCallback> for ReactionA {
    fn trigger(self, ctx: &mut runtime::Context, state: &mut <AsyncCallback as Reactor>::State) {
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
            state.expected_time += runtime::Duration::from_millis(200);
        } else {
            state.toggle = true;
        }
    }
}

#[derive(Reaction)]
#[reaction(reactor = "AsyncCallback", triggers(shutdown))]
struct ReactionShutdown;

impl Trigger<AsyncCallback> for ReactionShutdown {
    fn trigger(self, _ctx: &mut runtime::Context, state: &mut <AsyncCallback as Reactor>::State) {
        // make sure to join the thread before shutting down
        if state.thread.is_some() {
            std::mem::take(&mut state.thread).unwrap().join().unwrap();
        }
    }
}

#[test]
fn async_callback() {
    tracing_subscriber::fmt::init();
    let _ = boomerang_util::run::build_and_test_reactor::<AsyncCallback>(
        "async_callback",
        State {
            thread: None,
            expected_time: runtime::Duration::from_millis(100),
            toggle: false,
            i: 0,
        },
        false,
        true,
    )
    .unwrap();
}
