//! Events are scheduled with increasing additional delays of 0, 100, 300, 600 msec on a logical action with a minimum delay of 100 msec.  
//!
//! The use of the logical action ensures the elapsed time jumps exactly from 0 to 100, 300, 600, and 1000 msec.
//!
//! Ported from https://github.com/lf-lang/lingua-franca/blob/master/test/C/src/SlowingClock.lf
use boomerang::prelude::*;

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug)]
struct SlowingClock {
    interval: Duration,
    expected_time: Duration,
}

impl Default for SlowingClock {
    fn default() -> Self {
        Self {
            interval: Duration::milliseconds(100),
            expected_time: Duration::milliseconds(100),
        }
    }
}

#[derive(Reactor)]
#[reactor(
    state = "SlowingClock",
    reaction = "ReactionStartup",
    reaction = "ReactionA",
    reaction = "ReactionShutdown"
)]
struct SlowingClockBuilder {
    #[reactor(action(min_delay = "100 msec"))]
    a: TypedActionKey<()>,
}

#[derive(Reaction)]
#[reaction(reactor = "SlowingClockBuilder", triggers(startup))]
struct ReactionStartup<'a> {
    a: runtime::ActionRef<'a>,
}

impl runtime::Trigger<SlowingClock> for ReactionStartup<'_> {
    fn trigger(mut self, ctx: &mut runtime::Context, _state: &mut SlowingClock) {
        println!("startup");
        ctx.schedule_action(&mut self.a, (), None);
    }
}

#[derive(Reaction)]
#[reaction(reactor = "SlowingClockBuilder")]
struct ReactionA<'a> {
    #[reaction(triggers)]
    a: runtime::ActionRef<'a>,
}

impl runtime::Trigger<SlowingClock> for ReactionA<'_> {
    fn trigger(mut self, ctx: &mut runtime::Context, state: &mut SlowingClock) {
        let elapsed_logical_time = ctx.get_elapsed_logical_time();
        println!("Logical time since start: {elapsed_logical_time}.",);
        assert!(
            elapsed_logical_time == state.expected_time,
            "ERROR: Expected time to be: {}.",
            state.expected_time
        );

        ctx.schedule_action(&mut self.a, (), Some(state.interval));
        state.expected_time += Duration::milliseconds(100) + state.interval;
        state.interval += Duration::milliseconds(100);
    }
}

#[derive(Reaction)]
#[reaction(reactor = "SlowingClockBuilder", triggers(shutdown))]
struct ReactionShutdown;

impl runtime::Trigger<SlowingClock> for ReactionShutdown {
    fn trigger(self, _ctx: &mut runtime::Context, state: &mut SlowingClock) {
        assert_eq!(
            state.expected_time.whole_milliseconds(),
            1500,
            "ERROR: Expected the next expected_time to be: 1500ms. It was: {}.",
            state.expected_time
        );
        println!("Test passes.");
    }
}

#[test]
fn slowing_clock() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default()
        .with_fast_forward(true)
        .with_timeout(Duration::milliseconds(1000));
    let _ = boomerang_util::runner::build_and_test_reactor::<SlowingClockBuilder>(
        "slowing_clock",
        SlowingClock::default(),
        config,
    )
    .unwrap();
}
