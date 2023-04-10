//! Test asynchronous callbacks that trigger a physical action.

use boomerang::{
    builder::{BuilderReactionKey, Physical, TypedActionKey},
    runtime, Reactor,
};
use boomerang_util::{Timeout, TimeoutBuilder};
use std::thread::JoinHandle;

#[derive(Reactor)]
#[reactor(state = "AsyncCallback")]
struct AsyncCallbackBuilder {
    #[reactor(timer(period = "200 msec"))]
    t: TypedActionKey<()>,

    #[reactor(action(physical))]
    a: TypedActionKey<usize, Physical>,

    #[reactor(reaction(function = "AsyncCallback::reaction_t"))]
    reaction_t: BuilderReactionKey,

    #[reactor(reaction(function = "AsyncCallback::reaction_a"))]
    reaction_a: BuilderReactionKey,

    #[reactor(reaction(function = "AsyncCallback::reaction_shutdown"))]
    reaction_shutdown: BuilderReactionKey,

    #[reactor(child(state = "Timeout::new(runtime::Duration::from_secs(2))"))]
    _timeout: TimeoutBuilder,
}

struct AsyncCallback {
    thread: Option<JoinHandle<()>>,
    expected_time: runtime::Duration,
    toggle: bool,
    i: usize,
}

impl AsyncCallback {
    #[boomerang::reaction(reactor = "AsyncCallbackBuilder", triggers(action = "t"))]
    fn reaction_t(
        &mut self,
        ctx: &mut runtime::Context,
        #[reactor::action(effects)] mut a: runtime::PhysicalActionRef<usize>,
    ) {
        // make sure to join the old thread first
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }

        let mut send_ctx = ctx.make_send_context();

        // start new thread
        self.thread = Some(std::thread::spawn(move || {
            // Simulate time passing before a callback occurs
            std::thread::sleep(runtime::Duration::from_millis(100));
            // Schedule twice. If the action is not physical, these should get consolidated into a single action
            // triggering. If it is, then they cause two separate triggerings with close but not equal time stamps.
            send_ctx.schedule_action(&mut a, Some(0), None);
            send_ctx.schedule_action(&mut a, Some(0), None);
        }));
    }

    #[boomerang::reaction(reactor = "AsyncCallbackBuilder", triggers(action = "a"))]
    fn reaction_a(&mut self, ctx: &mut runtime::Context) {
        let elapsed_time = ctx.get_elapsed_logical_time();
        self.i += 1;
        tracing::info!(
            "Asynchronous callback {}: Assigned logical time greater than start time by {elapsed_time:?}",
            self.i,
        );
        if elapsed_time <= self.expected_time {
            panic!(
                "ERROR: Expected logical time to be larger than {:?}, was {elapsed_time:?}",
                self.expected_time
            );
        }
        if self.toggle {
            self.toggle = false;
            self.expected_time += runtime::Duration::from_millis(200);
        } else {
            self.toggle = true;
        }
    }

    #[boomerang::reaction(reactor = "AsyncCallbackBuilder", triggers(shutdown))]
    fn reaction_shutdown(&mut self, _ctx: &mut runtime::Context) {
        // make sure to join the thread before shutting down
        if self.thread.is_some() {
            std::mem::take(&mut self.thread).unwrap().join().unwrap();
        }
    }
}

#[test]
fn async_callback() {
    tracing_subscriber::fmt::init();
    let _ = boomerang_util::run::build_and_test_reactor::<AsyncCallbackBuilder>(
        "async_callback",
        AsyncCallback {
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
