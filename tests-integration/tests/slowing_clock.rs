/// Events are scheduled with increasing additional delays of 0, 100, 300, 600 msec on a logical action with a minimum delay of 100 msec.  
///
/// The use of the logical action ensures the elapsed time jumps exactly from 0 to 100, 300, 600, and 1000 msec.
///
/// Ported from https://github.com/lf-lang/lingua-franca/blob/master/test/C/src/SlowingClock.lf
use boomerang::{builder::prelude::*, runtime, Reaction, Reactor};
use boomerang_util::timeout;
use runtime::Duration;

struct SlowingClock {
    interval: Duration,
    expected_time: Duration,
}

impl Default for SlowingClock {
    fn default() -> Self {
        Self {
            interval: Duration::from_millis(100),
            expected_time: Duration::from_millis(100),
        }
    }
}

#[derive(Clone, Reactor)]
#[reactor(state = SlowingClock)]
struct SlowingClockBuilder {
    #[reactor(action(min_delay = "100 msec"))]
    a: TypedActionKey<()>,

    reaction_startup: TypedReactionKey<ReactionStartup<'static>>,
    reaction_a: TypedReactionKey<ReactionA<'static>>,
    reaction_shutdown: TypedReactionKey<ReactionShutdown>,

    #[reactor(child= runtime::Duration::from_secs(1))]
    _timeout: timeout::Timeout,
}

#[derive(Reaction)]
#[reaction(triggers(startup))]
struct ReactionStartup<'a> {
    a: runtime::ActionRef<'a>,
}

impl Trigger for ReactionStartup<'_> {
    type Reactor = SlowingClockBuilder;
    fn trigger(&mut self, ctx: &mut runtime::Context, _state: &mut SlowingClock) {
        println!("startup");
        ctx.schedule_action(&mut self.a, None, None);
    }
}

#[derive(Reaction)]
struct ReactionA<'a> {
    #[reaction(triggers)]
    a: runtime::ActionRef<'a>,
}

impl Trigger for ReactionA<'_> {
    type Reactor = SlowingClockBuilder;
    fn trigger(&mut self, ctx: &mut runtime::Context, state: &mut SlowingClock) {
        let elapsed_logical_time = ctx.get_elapsed_logical_time();
        println!(
            "Logical time since start: {}ms.",
            elapsed_logical_time.as_millis()
        );
        assert!(
            elapsed_logical_time == state.expected_time,
            "ERROR: Expected time to be: {}ms.",
            state.expected_time.as_millis()
        );

        ctx.schedule_action(&mut self.a, None, Some(state.interval));
        state.expected_time += Duration::from_millis(100) + state.interval;
        state.interval += Duration::from_millis(100);
    }
}

#[derive(Reaction)]
#[reaction(triggers(shutdown))]
struct ReactionShutdown;

impl Trigger for ReactionShutdown {
    type Reactor = SlowingClockBuilder;
    fn trigger(&mut self, _ctx: &mut runtime::Context, state: &mut SlowingClock) {
        assert_eq!(
            state.expected_time,
            Duration::from_millis(1500),
            "ERROR: Expected the next expected_time to be: 1500ms. It was: {}ms.",
            state.expected_time.as_millis()
        );
        println!("Test passes.");
    }
}

#[test]
fn slowing_clock() {
    tracing_subscriber::fmt::init();
    let _ = boomerang_util::run::build_and_test_reactor::<SlowingClockBuilder>(
        "slowing_clock",
        SlowingClock::default(),
        true,
        false,
    )
    .unwrap();
}