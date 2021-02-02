use boomerang::{
    builder::{self, EmptyPart, Reactor},
    runtime::{self, Duration, Instant},
};
use builder::ReactorPart;
use runtime::ActionKey;
use tracing::event;

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
// 	 	std::cout << "***** action " << count << " at time "
//                   << time << std::endl;
//         auto diff = time - previous_time;
//         if (diff != 200ms) {
//             std::cerr << "FAILURE: Expected 200 msecs of logical time to elapse "
//                       << "but got " << diff << std::endl;
//             exit(1);
//        }
//    =}
//}
// reactor Inside(period:time(1 sec),
//               message:{=std::string=}("Composite default message.")) {
//    third_instance = new HelloCpp(period = period, message = message);
//}
// main reactor Hello {
//    first_instance = new HelloCpp(period = 4 sec,
//                                  message = "Hello from first_instance.");
//    second_instance = new HelloCpp(message = "Hello from second_instance.");
//    composite_instance = new Inside(message = "Hello from composite_instance.");
//}

struct Hello {
    period: Duration,
    message: String,
    count: usize,
    previous_time: Instant,
}

impl Hello {
    fn new(period: Duration, message: &str) -> Self {
        Self {
            period,
            message: message.to_owned(),
            count: 0,
            previous_time: Instant::now(),
        }
    }

    /// reaction_t is sensitive to Timer `t` and schedules Action `a`
    fn reaction_t(
        &mut self,
        sched: &runtime::SchedulerPoint,
        _inputs: &<Self as Reactor>::Inputs,
        _outputs: &<Self as Reactor>::Outputs,
        actions: &<Self as Reactor>::Actions,
    ) {
        // Print the current time.
        self.previous_time = *sched.get_logical_time().get_time_point();
        sched.schedule_action(actions.a, (), Duration::from_millis(200)); // No payload.
        event!(tracing::Level::INFO, ?self.message, "Current time is {:?}", self.previous_time);
    }

    /// reaction_a is sensetive to Action `a`
    fn reaction_a(
        &mut self,
        sched: &runtime::SchedulerPoint,
        _inputs: &<Self as Reactor>::Inputs,
        _outputs: &<Self as Reactor>::Outputs,
        _actions: &<Self as Reactor>::Actions,
    ) {
        self.count += 1;
        let time = sched.get_logical_time();
        event!(
            tracing::Level::INFO,
            "***** action {} at time {}",
            self.count,
            time
        );
        let diff = *time.get_time_point() - self.previous_time;
        assert_eq!(
            diff,
            Duration::from_millis(200),
            "FAILURE: Expected 200 msecs of logical time to elapse but got {:?}",
            diff
        );
    }
}

#[derive(Clone)]
struct HelloActions {
    a: ActionKey<()>,
}
impl ReactorPart for HelloActions {
    fn build(
        env: &mut builder::EnvBuilder,
        reactor_key: runtime::ReactorKey,
    ) -> Result<Self, builder::BuilderError> {
        let a = env.add_logical_action("a", None, reactor_key)?;
        Ok(Self { a })
    }
}

impl Reactor for Hello {
    type Inputs = EmptyPart;
    type Outputs = EmptyPart;
    type Actions = HelloActions;

    fn build(
        self,
        name: &str,
        env: &mut boomerang::builder::EnvBuilder,
        parent: Option<boomerang::runtime::ReactorKey>,
    ) -> Result<
        (boomerang::runtime::ReactorKey, Self::Inputs, Self::Outputs),
        boomerang::builder::BuilderError,
    > {
        let period = self.period;
        let mut builder = env.add_reactor(name, parent, self);
        let t = builder.add_timer("t", period, Duration::from_micros(0))?;

        let Self::Actions { a } = builder.actions;
        let _ = builder
            .add_reaction(Self::reaction_t)
            .with_trigger_action(t)
            .with_scheduable_action(a.into())
            .finish();

        let _ = builder
            .add_reaction(Self::reaction_a)
            .with_trigger_action(a.into())
            .finish();

        builder.finish()
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
impl Reactor for Inside {
    type Inputs = EmptyPart;
    type Outputs = EmptyPart;
    type Actions = EmptyPart;

    fn build(
        self,
        name: &str,
        env: &mut builder::EnvBuilder,
        parent: Option<runtime::ReactorKey>,
    ) -> Result<(runtime::ReactorKey, Self::Inputs, Self::Outputs), builder::BuilderError> {
        let hello = Hello::new(Duration::from_secs(1), &self.message);
        let (key, inputs, outputs) = env.add_reactor(name, parent, self).finish()?;
        let (_, _, _) = hello.build("first_instance", env, Some(key))?;
        Ok((key, inputs, outputs))
    }
}

struct Main;
impl Reactor for Main {
    type Inputs = EmptyPart;
    type Outputs = EmptyPart;
    type Actions = EmptyPart;

    fn build(
        self,
        name: &str,
        env: &mut boomerang::builder::EnvBuilder,
        parent: Option<boomerang::runtime::ReactorKey>,
    ) -> Result<
        (boomerang::runtime::ReactorKey, Self::Inputs, Self::Outputs),
        boomerang::builder::BuilderError,
    > {
        let (key, inputs, outputs) = env.add_reactor(name, parent, self).finish()?;

        let (_, _, _) = Hello::new(Duration::from_secs(4), "Hello from first_instance.").build(
            "first_instance",
            env,
            Some(key),
        )?;

        let (_, _, _) = Hello::new(Duration::from_secs(1), "Hello from second_instance.").build(
            "second_instance",
            env,
            Some(key),
        )?;

        let (_, _, _) = Inside::new("Hello from composite_instance.").build(
            "composite_instance",
            env,
            Some(key),
        )?;

        Ok((key, inputs, outputs))
    }
}

#[test]
fn hello() {
    // install global collector configured based on RUST_LOG env var.
    tracing_subscriber::fmt::init();
    use std::convert::TryInto;

    use boomerang::builder::*;
    let mut env_builder = EnvBuilder::new();
    let (_, _, _) = Main {}.build("main", &mut env_builder, None).unwrap();

    let env: runtime::Environment = env_builder.try_into().unwrap();

    for port in env.ports.values() {
        println!("{}", port);
    }

    for action in env.actions.values() {
        println!("{}", action);
    }
}
