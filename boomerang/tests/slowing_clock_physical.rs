//! In this example, events are scheduled with increasing additional delays of 0, 100, 300, 600 msec
//! on a physical action with a minimum delay of 100 msec.  The use of the physical action makes the
//! elapsed time jumps from 0 to approximately 100 msec, to approximatly 300 msec thereafter,
//! drifting away further with each new event. Modeled after the Lingua-Franca C version of this
//! test. @author Maiko Brants TU Dresden

use boomerang::prelude::*;

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
}

#[derive(Reaction)]
#[reaction(reactor = "SlowingClockPhysical", triggers(startup))]
struct ReactionStartup<'a> {
    a: runtime::ActionRef<'a>,
}

impl runtime::Trigger<State> for ReactionStartup<'_> {
    fn trigger(mut self, ctx: &mut runtime::Context, state: &mut State) {
        state.expected_time = Duration::milliseconds(100);
        ctx.schedule_action(&mut self.a, (), None);
    }
}

#[derive(Reaction)]
#[reaction(reactor = "SlowingClockPhysical")]
struct ReactionA<'a> {
    #[reaction(triggers)]
    a: runtime::ActionRef<'a>,
}

impl runtime::Trigger<State> for ReactionA<'_> {
    fn trigger(mut self, ctx: &mut runtime::Context, state: &mut State) {
        let elapsed_logical_time = ctx.get_elapsed_logical_time();
        println!("Logical time since start: {elapsed_logical_time:?}");
        assert!(
            elapsed_logical_time >= state.expected_time,
            "Expected logical time to be at least: {:?}, was {elapsed_logical_time:?}",
            state.expected_time
        );
        state.interval += Duration::milliseconds(100);
        state.expected_time = Duration::milliseconds(100) + state.interval;
        println!(
            "Scheduling next to occur approximately after: {:?}",
            state.interval
        );
        ctx.schedule_action(&mut self.a, (), Some(state.interval));
    }
}

#[derive(Reaction)]
#[reaction(reactor = "SlowingClockPhysical", triggers(shutdown))]
struct ReactionShutdown;

impl runtime::Trigger<State> for ReactionShutdown {
    fn trigger(self, _ctx: &mut runtime::Context, state: &mut State) {
        assert!(
            state.expected_time >= Duration::milliseconds(500),
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
        .with_keep_alive(true)
        .with_timeout(Duration::milliseconds(1500));
    let _ = boomerang_util::runner::build_and_test_reactor::<SlowingClockPhysical>(
        "slowing_clock_physical",
        State {
            interval: Duration::milliseconds(100),
            expected_time: Duration::milliseconds(200),
        },
        config,
    )
    .unwrap();
}
