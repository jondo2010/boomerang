/// Events are scheduled with increasing additional delays of 0, 100, 300, 600
/// msec on a logical action with a minimum delay of 100 msec.  
/// The use of the logical action ensures the elapsed time jumps exactly from
/// 0 to 100, 300, 600, and 1000 msec.
use boomerang::{builder::BuilderActionKey, runtime, Reactor, boomerang_test_body};
use boomerang_util::{Timeout, TimeoutBuilder};
use runtime::Duration;

#[derive(Reactor)]
struct SlowingClockBuilder {
    #[reactor(action(min_delay = "100 msec"))]
    a: BuilderActionKey,
    #[reactor(reaction(function = "SlowingClock::reaction_startup"))]
    reaction_startup: runtime::ReactionKey,
    #[reactor(reaction(function = "SlowingClock::reaction_a"))]
    reaction_a: runtime::ReactionKey,
    #[reactor(reaction(function = "SlowingClock::reaction_shutdown"))]
    reaction_shutdown: runtime::ReactionKey,
    #[reactor(child(state = "Timeout::new(runtime::Duration::from_secs(1))"))]
    _timeout: TimeoutBuilder,
}

struct SlowingClock {
    interval: Duration,
    expected_time: Duration,
}

impl SlowingClock {
    fn new() -> Self {
        SlowingClock {
            interval: Duration::from_millis(100),
            expected_time: Duration::from_millis(100),
        }
    }

    #[boomerang::reaction(reactor = "SlowingClockBuilder", triggers(startup))]
    fn reaction_startup(
        &mut self,
        ctx: &mut runtime::Context,
        #[reactor::action(effects)] mut a: runtime::ActionMut,
    ) {
        println!("startup");
        ctx.schedule_action::<()>(&mut a, None, None);
    }

    #[boomerang::reaction(reactor = "SlowingClockBuilder", triggers(action = "a"))]
    fn reaction_a(
        &mut self,
        ctx: &mut runtime::Context,
        #[reactor::action(effects)] mut a: runtime::ActionMut,
    ) {
        let elapsed_logical_time = ctx.get_elapsed_logical_time();
        println!(
            "Logical time since start: {}ms.",
            elapsed_logical_time.as_millis()
        );
        assert!(
            elapsed_logical_time == self.expected_time,
            "ERROR: Expected time to be: {}ms.",
            self.expected_time.as_millis()
        );

        ctx.schedule_action::<()>(&mut a, None, Some(self.interval));
        self.expected_time += Duration::from_millis(100) + self.interval;
        self.interval += Duration::from_millis(100);
    }

    #[boomerang::reaction(reactor = "SlowingClockBuilder", triggers(shutdown))]
    fn reaction_shutdown(&mut self, _ctx: &mut runtime::Context) {
        assert_eq!(
            self.expected_time,
            Duration::from_millis(1500),
            "ERROR: Expected the next expected_time to be: 1500ms. It was: {}ms.",
            self.expected_time.as_millis()
        );
        println!("Test passes.");
    }
}

boomerang_test_body!(slowing_clock, SlowingClockBuilder, SlowingClock::new());