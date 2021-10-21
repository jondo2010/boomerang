use boomerang::{
    builder::{self, BuilderError, EmptyPart, Reactor},
    runtime::{self, Duration, ReactorKey},
    ReactorActions,
};
use runtime::{Config, RunMode, SchedulerPoint};

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

    /// reaction_t is sensitive to Timer `t` and schedules Action `a`
    fn reaction_t<S: SchedulerPoint>(
        &mut self,
        sched: &S,
        _inputs: &EmptyPart,
        _outputs: &EmptyPart,
        actions: &HelloActions,
    ) {
        // Print the current time.
        self.previous_time = sched.get_elapsed_logical_time();
        sched.schedule_action(actions.a, (), Some(Duration::from_millis(200))); // No payload.
        println!("{} Current time is {:?}", self.message, self.previous_time);
    }

    /// reaction_a is sensetive to Action `a`
    fn reaction_a<S: SchedulerPoint>(
        &mut self,
        sched: &S,
        _inputs: &EmptyPart,
        _outputs: &EmptyPart,
        _actions: &HelloActions,
    ) {
        self.count += 1;
        let time = sched.get_elapsed_logical_time();
        println!("***** action {} at time {:?}", self.count, time);
        let diff = time - self.previous_time;
        assert_eq!(
            diff,
            Duration::from_millis(200),
            "FAILURE: Expected 200 msecs of logical time to elapse but got {:?}",
            diff
        );
    }
}

ReactorActions!(Hello, HelloActions, (a, (), None));
impl<S: SchedulerPoint> Reactor<S> for Hello {
    type Inputs = EmptyPart;
    type Outputs = EmptyPart;
    type Actions = HelloActions;

    fn build(
        self,
        name: &str,
        env: &mut boomerang::builder::EnvBuilder<S>,
        parent: Option<ReactorKey>,
    ) -> Result<(ReactorKey, Self::Inputs, Self::Outputs), BuilderError> {
        let period = self.period;
        let mut builder = env.add_reactor(name, parent, self);
        let t = builder.add_timer("t", period, Duration::from_secs(1))?;

        let Self::Actions { a } = builder.actions;
        let _ = builder
            .add_reaction("reaction_t", Self::reaction_t)
            .with_trigger_action(t)
            .with_scheduable_action(a.into())
            .finish()?;

        let _ = builder
            .add_reaction("reaction_a", Self::reaction_a)
            .with_trigger_action(a.into())
            .finish()?;

        builder.finish()
    }

    fn build_parts(
        &self,
        env: &mut builder::EnvBuilder<S>,
        reactor_key: ReactorKey,
    ) -> Result<(Self::Inputs, Self::Outputs, Self::Actions), BuilderError> {
        Ok((EmptyPart, EmptyPart, HelloActions::build(env, reactor_key)?))
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
impl<S: SchedulerPoint> Reactor<S> for Inside {
    type Inputs = EmptyPart;
    type Outputs = EmptyPart;
    type Actions = EmptyPart;

    fn build(
        self,
        name: &str,
        env: &mut builder::EnvBuilder<S>,
        parent: Option<runtime::ReactorKey>,
    ) -> Result<(runtime::ReactorKey, Self::Inputs, Self::Outputs), builder::BuilderError> {
        let hello = Hello::new(Duration::from_secs(1), &self.message);
        let (key, inputs, outputs) = env.add_reactor(name, parent, self).finish()?;
        let (_, _, _) = hello.build("first_instance", env, Some(key))?;
        Ok((key, inputs, outputs))
    }

    fn build_parts(
        &self,
        _env: &mut builder::EnvBuilder<S>,
        _reactor_key: ReactorKey,
    ) -> Result<(Self::Inputs, Self::Outputs, Self::Actions), BuilderError> {
        Ok((EmptyPart, EmptyPart, EmptyPart))
    }
}

struct Main;
impl<S: SchedulerPoint> Reactor<S> for Main {
    type Inputs = EmptyPart;
    type Outputs = EmptyPart;
    type Actions = EmptyPart;

    fn build(
        self,
        name: &str,
        env: &mut boomerang::builder::EnvBuilder<S>,
        parent: Option<ReactorKey>,
    ) -> Result<(ReactorKey, Self::Inputs, Self::Outputs), BuilderError> {
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

    fn build_parts(
        &self,
        _env: &mut builder::EnvBuilder<S>,
        _reactor_key: ReactorKey,
    ) -> Result<(Self::Inputs, Self::Outputs, Self::Actions), BuilderError> {
        Ok((EmptyPart, EmptyPart, EmptyPart))
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

    let gv = graphviz::build(&env_builder).unwrap();
    let mut f = std::fs::File::create(format!("{}.dot", module_path!())).unwrap();
    std::io::Write::write_all(&mut f, gv.as_bytes()).unwrap();

    let gv = graphviz::build_reaction_graph(&env_builder).unwrap();
    let mut f = std::fs::File::create(format!("{}_levels.dot", module_path!())).unwrap();
    std::io::Write::write_all(&mut f, gv.as_bytes()).unwrap();

    let env: runtime::Env<_> = env_builder.try_into().unwrap();
    let mut sched = runtime::Scheduler::with_config(
        env,
        Config::new(RunMode::RunFor(Duration::from_secs(10)), true),
    );
    sched.start().unwrap();
}
