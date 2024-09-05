#![allow(dead_code)]

use std::time::Duration;

use boomerang::{builder::prelude::*, runtime, Reaction, Reactor};
use boomerang_util::timeout;

// This test checks that logical time is incremented an appropriate
// amount as a result of an invocation of the schedule() function at
// runtime. It also performs various smoke tests of timing aligned
// reactions. The first instance has a period of 4 seconds, the second
// of 2 seconds, and the third (composite) or 1 second.

// reactor HelloCpp(period:time(2 secs), message:{=std::string=}("Hello C++")) {
//    state count:int(0);
//    state previous_time:{=reactor::TimePoint=}();
//    timer t(1 secs, period);
//    logical action a:void;
//    reaction(t) -> a {=
//        std::cout << message << std::endl;
//        a.schedule(200ms); // No payload.
//        // Print the current time.
//        previous_time = get_logical_time();
//        std::cout << "Current time is " << previous_time << std::endl;
//     =}
//    reaction(a) {=
//         count++;
//         auto time = get_logical_time();
// 	 	std::cout << "***** action " << count << " at time " << time << std::endl;
//         auto diff = time - previous_time;
//         if (diff != 200ms) {
//             std::cerr << "FAILURE: Expected 200 msecs of logical time to elapse " << "but got "
// << diff << std::endl;             exit(1);
//        }
//    =}
//}
// reactor Inside(period:time(1 sec), message:{=std::string=}("Composite default message.")) {
//    third_instance = new HelloCpp(period = period, message = message);
//}
// main reactor Hello {
//    first_instance = new HelloCpp(period = 4 sec, message = "Hello from first_instance.");
//    second_instance = new HelloCpp(message = "Hello from second_instance.");
//    composite_instance = new Inside(message = "Hello from composite_instance.");
//}

struct Hello {
    period: runtime::Duration,
    message: String,
    count: usize,
    previous_time: runtime::Duration,
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

#[derive(Clone, Reactor)]
#[reactor(state = Hello)]
struct HelloBuilder {
    #[reactor(timer(offset = "1 sec", period = "2 sec"))]
    t: TimerActionKey,
    #[reactor(action())]
    a: TypedActionKey<()>,
    reaction_t: TypedReactionKey<ReactionT<'static>>,
    reaction_a: TypedReactionKey<ReactionA>,
}

/// ReactionT is sensitive to Timer `t` and schedules Action `a`
#[derive(Reaction)]
#[reaction(triggers(action = "t"))]
struct ReactionT<'a> {
    a: runtime::ActionRef<'a, ()>,
}

impl Trigger for ReactionT<'_> {
    type Reactor = HelloBuilder;
    fn trigger(&mut self, ctx: &mut runtime::Context, state: &mut Hello) {
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
#[reaction(triggers(action = "a"))]
struct ReactionA;

impl Trigger for ReactionA {
    type Reactor = HelloBuilder;
    fn trigger(&mut self, ctx: &mut runtime::Context, state: &mut Hello) {
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

#[derive(Clone, Reactor)]
#[reactor(state = Inside)]
struct InsideBuilder {
    #[reactor(child = Hello::new(Duration::from_secs(1), "Composite default message."))]
    _third_instance: HelloBuilder,
}

#[derive(Clone, Reactor)]
#[reactor(state = ())]
struct MainBuilder {
    #[reactor(child = Hello::new(Duration::from_secs(4), "Hello from first."))]
    first_instance: HelloBuilder,
    #[reactor(child = Hello::new(Duration::from_secs(2), "Hello from second."))]
    second_instance: HelloBuilder,
    #[reactor(child = Inside::new("Hello from composite."))]
    third_instance: InsideBuilder,

    #[reactor(child = runtime::Duration::from_secs(10))]
    _timeout: timeout::Timeout,
}

#[test]
fn hello() {
    tracing_subscriber::fmt::init();
    let _ = boomerang_util::run::build_and_test_reactor::<MainBuilder>("hello", (), true, false)
        .unwrap();
}