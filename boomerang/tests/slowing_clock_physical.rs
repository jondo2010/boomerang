//! In this example, events are scheduled with increasing additional delays of 0, 100, 300, 600 msec
//! on a physical action with a minimum delay of 100 msec.  The use of the physical action makes the
//! elapsed time jumps from 0 to approximately 100 msec, to approximatly 300 msec thereafter,
//! drifting away further with each new event. Modeled after the Lingua-Franca C version of this
//! test. @author Maiko Brants TU Dresden

use std::time::Duration;

use boomerang::{
    builder::{BuilderReactionKey, Physical, TypedActionKey},
    runtime, Reactor,
};
use boomerang_util::{Timeout, TimeoutBuilder};
use tracing::info;

#[derive(Reactor)]
#[reactor(state = "SlowingClockPhysical")]
struct SlowingClockPhysicalBuilder {
    #[reactor(action(physical, min_delay = "100 msec"))]
    a: TypedActionKey<(), Physical>,

    #[reactor(reaction(function = "SlowingClockPhysical::reaction_startup"))]
    startup: BuilderReactionKey,

    #[reactor(reaction(function = "SlowingClockPhysical::reaction_a"))]
    r_a: BuilderReactionKey,

    #[reactor(reaction(function = "SlowingClockPhysical::reaction_shutdown"))]
    shutdown: BuilderReactionKey,

    #[reactor(child(state = "Timeout::new(Duration::from_millis(1500))"))]
    _timeout: TimeoutBuilder,
}

#[derive(Clone)]
struct SlowingClockPhysical {
    interval: Duration,
    expected_time: Duration,
}

impl SlowingClockPhysical {
    #[boomerang::reaction(reactor = "SlowingClockPhysicalBuilder", triggers(startup))]
    fn reaction_startup(
        &mut self,
        ctx: &mut runtime::Context,
        #[reactor::action(effects)] mut a: runtime::PhysicalActionRef,
    ) {
        self.expected_time = Duration::from_millis(100);
        ctx.schedule_action(&mut a, None, None);
    }

    #[boomerang::reaction(reactor = "SlowingClockPhysicalBuilder")]
    fn reaction_a(
        &mut self,
        ctx: &mut runtime::Context,
        #[reactor::action(effects, triggers)] mut a: runtime::PhysicalActionRef,
    ) {
        let elapsed_logical_time = ctx.get_elapsed_logical_time();
        info!("Logical time since start: {elapsed_logical_time:?}");
        assert!(
            elapsed_logical_time >= self.expected_time,
            "Expected logical time to be at least: {:?}, was {elapsed_logical_time:?}",
            self.expected_time
        );
        self.interval += Duration::from_millis(100);
        self.expected_time = Duration::from_millis(100) + self.interval;
        info!(
            "Scheduling next to occur approximately after: {:?}",
            self.interval
        );
        ctx.schedule_action(&mut a, None, Some(self.interval));
    }

    #[boomerang::reaction(reactor = "SlowingClockPhysicalBuilder", triggers(shutdown))]
    fn reaction_shutdown(&mut self, _ctx: &mut runtime::Context) {
        assert!(
            self.expected_time >= Duration::from_millis(500),
            "Expected the next expected time to be at least: 500000000 nsec. It was: {:?}",
            self.expected_time
        );
    }
}

#[test_log::test]
fn slowing_clock_physical() {
    let _ = boomerang_util::run::build_and_test_reactor::<SlowingClockPhysicalBuilder>(
        "slowing_clock_physical",
        SlowingClockPhysical {
            interval: Duration::from_millis(100),
            expected_time: Duration::from_millis(200),
        },
        true,
        true,
    )
    .unwrap();
}
