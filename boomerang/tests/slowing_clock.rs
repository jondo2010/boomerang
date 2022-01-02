/// Events are scheduled with increasing additional delays of 0, 100, 300, 600
/// msec on a logical action with a minimum delay of 100 msec.  
/// The use of the logical action ensures the elapsed time jumps exactly from
/// 0 to 100, 300, 600, and 1000 msec.
use boomerang::{runtime, Reactor};
use runtime::{Duration, InternalAction};

#[derive(Debug, Reactor)]
#[reactor(reaction(function = "Timeout::reaction_startup", triggers(startup)))]
struct Timeout {
    timeout: runtime::Duration,
}
impl Timeout {
    fn new(timeout: runtime::Duration) -> Self {
        Timeout { timeout }
    }
    fn reaction_startup(&mut self, ctx: &mut runtime::Context) {
        ctx.schedule_shutdown(Some(self.timeout))
    }
}

#[derive(Reactor, Debug)]
#[reactor(
    action(name = "a", min_delay = "100 msec"),
    reaction(
        function = "SlowingClock::reaction_startup",
        triggers(startup),
        effects(action = "a")
    ),
    reaction(
        function = "SlowingClock::reaction_a",
        triggers(action = "a"),
        effects(action = "a")
    ),
    reaction(function = "SlowingClock::reaction_shutdown", triggers(shutdown)),
    child(
        name = "timeout",
        reactor = "Timeout::new(runtime::Duration::from_secs(1))"
    )
)]
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

    fn reaction_startup(&mut self, ctx: &mut runtime::Context, a: &mut InternalAction) {
        println!("startup");
        ctx.schedule_action::<()>(a, None, None);
    }

    fn reaction_a(&mut self, ctx: &mut runtime::Context, a: &mut InternalAction) {
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

        ctx.schedule_action::<()>(a, None, Some(self.interval));
        self.expected_time += Duration::from_millis(100) + self.interval;
        self.interval += Duration::from_millis(100);
    }

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

#[cfg(feature = "disabled")]
#[test]
fn slowing_clock() {
    // install global collector configured based on RUST_LOG env var.
    tracing_subscriber::fmt::init();
    use std::convert::TryInto;

    use boomerang::builder::*;
    let mut env_builder = EnvBuilder::new();
    let (key, parts) = SlowingClock::new()
        .build("slowing_clock", &mut env_builder, None)
        .unwrap();

    let gv = graphviz::build(&env_builder).unwrap();
    let mut f = std::fs::File::create(format!("{}.dot", module_path!())).unwrap();
    std::io::Write::write_all(&mut f, gv.as_bytes()).unwrap();

    let (env, dep_info) = env_builder.try_into().unwrap();

    runtime::check_consistency(&env, &dep_info);

    let sched = runtime::Scheduler::new(env, dep_info, true);
    sched.event_loop();
}
