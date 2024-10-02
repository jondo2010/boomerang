//! This test checks that logical time is incremented an appropriate amount as a result of an invocation of the
//! schedule_action() function at runtime. It also performs various smoke tests of timing aligned reactions. The first
//! instance has a period of 4 seconds, the second of 2 seconds, and the third (composite) or 1 second.

use std::time::Duration;

use boomerang::prelude::*;
use boomerang_util::timeout;

struct Hello {
    period: Duration,
    message: String,
    count: usize,
    previous_time: Duration,
}

impl Hello {
    fn new(period: Duration, message: &str) -> Self {
        Self {
            period,
            message: message.to_owned(),
            count: 0,
            previous_time: Duration::default(),
        }
    }
}

#[derive(Reactor)]
#[reactor(state = "Hello", reaction = "ReactionT", reaction = "ReactionA")]
struct HelloBuilder {
    #[reactor(timer(offset = "1 sec", period = "2 sec"))]
    t: TimerActionKey,
    #[reactor(action())]
    a: TypedActionKey<()>,
}

/// ReactionT is sensitive to Timer `t` and schedules Action `a`
#[derive(Reaction)]
#[reaction(reactor = "HelloBuilder", triggers(action = "t"))]
struct ReactionT<'a> {
    a: runtime::ActionRef<'a, ()>,
}

impl Trigger<HelloBuilder> for ReactionT<'_> {
    fn trigger(mut self, ctx: &mut runtime::Context, state: &mut Hello) {
        // Print the current time.
        state.previous_time = ctx.get_elapsed_logical_time();
        ctx.schedule_action(&mut self.a, None, Some(Duration::from_millis(200))); // No payload.
        println!(
            "{} Current time is {:?}",
            state.message, state.previous_time
        );
    }
}

/// ReactionA is sensetive to Action `a`
#[derive(Reaction)]
#[reaction(reactor = "HelloBuilder", triggers(action = "a"))]
struct ReactionA;

impl Trigger<HelloBuilder> for ReactionA {
    fn trigger(self, ctx: &mut runtime::Context, state: &mut Hello) {
        state.count += 1;
        let time = ctx.get_elapsed_logical_time();
        println!("***** action {} at time {:?}", state.count, time);
        let diff = time - state.previous_time;
        assert_eq!(
            diff,
            Duration::from_millis(200),
            "FAILURE: Expected 200 msecs of logical time to elapse but got {:?}",
            diff
        );
    }
}

struct Inside {
    message: String,
}
impl Inside {
    fn new(message: &str) -> Self {
        Self {
            message: message.to_owned(),
        }
    }
}

#[derive(Reactor)]
#[reactor(state = Inside)]
struct InsideBuilder {
    #[reactor(child = Hello::new(Duration::from_secs(1), "Composite default message."))]
    _third_instance: HelloBuilder,
}

#[derive(Reactor)]
#[reactor(state = ())]
struct MainBuilder {
    #[reactor(child = Hello::new(Duration::from_secs(4), "Hello from first."))]
    first_instance: HelloBuilder,
    #[reactor(child = Hello::new(Duration::from_secs(2), "Hello from second."))]
    second_instance: HelloBuilder,
    #[reactor(child = Inside::new("Hello from composite."))]
    third_instance: InsideBuilder,

    #[reactor(child = Duration::from_secs(10))]
    _timeout: timeout::Timeout,
}

#[test]
fn hello() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default().with_fast_forward(true);
    let _ =
        boomerang_util::runner::build_and_test_reactor::<MainBuilder>("hello", (), config).unwrap();
}
