//! This checks that the after keyword adjusts logical time, not using physical time.

use std::time::Duration;

use boomerang::prelude::*;

#[derive(Reactor)]
#[reactor(
    state = "()",
    reaction = "ReactionFooX",
)]
struct Foo {
    x: TypedPortKey<i32, Input>,
    y: TypedPortKey<i32, Output>,
}

#[derive(Reaction)]
#[reaction(reactor = "Foo")]
struct ReactionFooX<'a> {
    x: runtime::InputRef<'a, i32>,
    y: runtime::OutputRef<'a, i32>,
}

impl runtime::Trigger<()> for ReactionFooX<'_> {
    fn trigger(mut self, _ctx: &mut runtime::Context, _state: &mut ()) {
        *self.y = *self.x;
    }
}

#[derive(Debug)]
struct PrintState {
    expected_time: Duration,
    i: usize,
}

impl Default for PrintState {
    fn default() -> Self {
        Self {
            expected_time: Duration::from_millis(10),
            i: 0,
        }
    }
}

#[derive(Reactor)]
#[reactor(
    state = "PrintState",
    reaction = "ReactionPrintX",
    reaction = "ReactionPrintShutdown",
)]
struct Print {
    x: TypedPortKey<i32, Input>,
}

#[derive(Reaction)]
#[reaction(reactor = "Print")]
struct ReactionPrintX<'a> {
    x: runtime::InputRef<'a, i32>,
}

impl runtime::Trigger<PrintState> for ReactionPrintX<'_> {
    fn trigger(self, ctx: &mut runtime::Context, state: &mut PrintState) {
        state.i += 1;
        let elapsed_time = ctx.get_elapsed_logical_time();
        println!("Result is {:?}", *self.x);
        assert_eq!(*self.x, Some(84), "Expected result to be 84");
        println!("Current logical time is: {:?}", elapsed_time);
        println!("Current physical time is: {:?}", ctx.get_physical_time());
        assert_eq!(elapsed_time, state.expected_time, "Expected logical time to be {:?}", state.expected_time);
        state.expected_time += Duration::from_secs(1);
    }
}

#[derive(Reaction)]
#[reaction(reactor = "Print")]
struct ReactionPrintShutdown<'a> {
    x: runtime::InputRef<'a, i32>,
}

impl runtime::Trigger<PrintState> for ReactionPrintShutdown<'_> {
    fn trigger(self, ctx: &mut runtime::Context, state: &mut PrintState) {
        assert_eq!(state.i, 1, "ERROR: Final reactor received no data.");
    }
}

#[derive(Reactor)]
#[reactor(
    state = "()",
    reaction = "ReactionMainT",
    connection(from = "f.y", to = "p.x", after = "10 msec")
)]
struct Main {
    #[reactor(child = "()")]
    f: Foo,
    #[reactor(child = "PrintState::default()")]
    p: Print,
    #[reactor(timer(period = "1 msec"))]
    t: TimerActionKey,
    #[reactor(child = "Duration::from_secs(3)")]
    _timeout: boomerang_util::timeout::Timeout,
}

#[derive(Reaction)]
#[reaction(reactor = "Main", triggers(action = "t"))]
struct ReactionMainT<'a> {
    #[reaction(path = "f.x")]
    x: runtime::OutputRef<'a, i32>,
}

impl runtime::Trigger<()> for ReactionMainT<'_> {
    fn trigger(mut self, _ctx: &mut runtime::Context, _state: &mut ()) {
        *self.x = Some(42);
        println!("Timer!");
    }
}

#[test]
fn after() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default().with_fast_forward(true);
    let _ = boomerang_util::runner::build_and_test_reactor::<Main>(
        "after",
        (),
        config,
    )
    .unwrap();
}
