//! In this example, events are scheduled with increasing additional delays of 0, 100, 300, 600 msec
//! on a physical action with a minimum delay of 100 msec.  The use of the physical action makes the
//! elapsed time jumps from 0 to approximately 100 msec, to approximatly 300 msec thereafter,
//! drifting away further with each new event. Modeled after the Lingua-Franca C version of this
//! test. @author Maiko Brants TU Dresden

use std::time::Duration;

use boomerang::prelude::*;
use boomerang_util::timeout;

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Default)]
struct State {
    interval: Duration,
    expected_time: Duration,
}

#[derive(Reactor)]
#[reactor(
    state = "State",
    reaction = "ReactionStartup",
    reaction = "ReactionA",
    reaction = "ReactionShutdown"
)]
struct SlowingClockPhysical {
    #[reactor(action(min_delay = "100 msec"))]
    a: TypedActionKey<(), Physical>,

    #[reactor(child = "Duration::from_millis(1500).into()")]
    _timeout: timeout::Timeout,
}

#[derive(Reaction)]
#[reaction(reactor = "SlowingClockPhysical", triggers(startup))]
struct ReactionStartup {
    a: runtime::PhysicalActionRef,
}

impl runtime::Trigger<State> for ReactionStartup {
    fn trigger(mut self, ctx: &mut runtime::Context, state: &mut State) {
        state.expected_time = Duration::from_millis(100);
        ctx.schedule_action(&mut self.a, None, None);
    }
}

#[derive(Reaction)]
#[reaction(reactor = "SlowingClockPhysical")]
struct ReactionA {
    #[reaction(triggers)]
    a: runtime::PhysicalActionRef,
}

impl runtime::Trigger<State> for ReactionA {
    fn trigger(mut self, ctx: &mut runtime::Context, state: &mut State) {
        let elapsed_logical_time = ctx.get_elapsed_logical_time();
        println!("Logical time since start: {elapsed_logical_time:?}");
        assert!(
            elapsed_logical_time >= state.expected_time,
            "Expected logical time to be at least: {:?}, was {elapsed_logical_time:?}",
            state.expected_time
        );
        state.interval += Duration::from_millis(100);
        state.expected_time = Duration::from_millis(100) + state.interval;
        println!(
            "Scheduling next to occur approximately after: {:?}",
            state.interval
        );
        ctx.schedule_action(&mut self.a, None, Some(state.interval));
    }
}

#[derive(Reaction)]
#[reaction(reactor = "SlowingClockPhysical", triggers(shutdown))]
struct ReactionShutdown;

impl runtime::Trigger<State> for ReactionShutdown {
    fn trigger(self, _ctx: &mut runtime::Context, state: &mut State) {
        assert!(
            state.expected_time >= Duration::from_millis(500),
            "Expected the next expected time to be at least: 500000000 nsec. It was: {:?}",
            state.expected_time
        );
    }
}

#[test]
fn slowing_clock_physical() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default()
        .with_fast_forward(true)
        .with_keep_alive(true);
    let _ = boomerang_util::runner::build_and_test_reactor::<SlowingClockPhysical>(
        "slowing_clock_physical",
        State {
            interval: Duration::from_millis(100),
            expected_time: Duration::from_millis(200),
        },
        config,
    )
    .unwrap();
}
