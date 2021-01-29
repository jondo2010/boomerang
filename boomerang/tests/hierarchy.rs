use core::num;
use std::sync::{Arc, RwLock};

use boomerang::{
    builder::{BuilderError, EnvBuilder, Reactor, ReactorBuilderState},
    runtime::{self, PortKey, ReactorKey},
};
use tracing::event;

// Test data transport across hierarchy.
// target Cpp;
// reactor Source {
//     output out:int;
//     timer t;
//     reaction(t) -> out {=
//         out.set(1);
//     =}
// }
// reactor Gain {
//     input in:int;
//     output out:int;
//     reaction(in) -> out {=
//         out.set((*in.get()) * 2);
//     =}
// }
// reactor Print {
//     input in:int;
//     reaction(in) {=
//         auto value = *in.get();
//         std::cout << "Received: " << value << std::endl;
//         if (value != 2) {
//             std::cerr << "Expected 2." << std::endl;
//             exit(1);
//         }
//     =}
// }
// reactor GainContainer {
//     input in:int;
//     output out:int;
//     output out2:int;
//     gain = new Gain();
//     in -> gain.in;
//     gain.out -> out;
//     gain.out -> out2;
// }
// main reactor Hierarchy {
//     source = new Source();
//     container = new GainContainer();
//     print = new Print();
//     print2 = new Print();
//     source.out -> container.in;
//     container.out -> print.in;
//     container.out -> print2.in;
// }

struct Source {
    value: u32,
}

impl Reactor for Source {
    type Inputs = ();
    type Outputs = (PortKey<u32>);

    fn build(
        self,
        name: &str,
        env: &mut EnvBuilder,
        parent: Option<ReactorKey>,
    ) -> Result<(ReactorKey, Self::Inputs, Self::Outputs), BuilderError> {
        let builder = env.add_reactor(name, parent, self);
        let out = builder.add_output::<u32>("out")?;
        let t = builder.add_startup_timer("t");

        let r0 = builder
            .add_reaction(Self::r0)
            .with_trigger_action(t)
            .with_antidependency(out)
            .finish(env);

        let key = builder.finish()?;
        Ok((key, (), (out)))
    }
}

impl Source {
    pub fn new(value: u32) -> Self {
        Self { value }
    }

    fn r0(&self, sched: &runtime::SchedulerPoint) {
        sched.set_port(self.out, 1u32);
        event!(
            tracing::Level::INFO,
            "Sent {:?}",
            sched.get_port::<u32>(self.out)
        );
    }
}

struct Gain {
    gain: u32,
}

impl Reactor for Gain {
    type Inputs = (PortKey<u32>);
    type Outputs = (PortKey<u32>);
    fn build(
        self,
        name: &str,
        env: &mut EnvBuilder,
        parent: Option<ReactorKey>,
    ) -> Result<(ReactorKey, Self::Inputs, Self::Outputs), BuilderError> {
        let builder = env.add_reactor(name, parent, self);

        let inp = builder.add_input::<u32>("in")?;
        let out = builder.add_output::<u32>("out")?;

        let _ = builder
            .add_reaction(Self::r0)
            .with_trigger_port(inp)
            .with_antidependency(out)
            .finish();

        let key = builder.finish()?;
        Ok((key, inp, out))
    }
}

impl Gain {
    pub fn new(gain: u32) -> Self {
        Self { gain }
    }
    fn r0(&self, sched: &runtime::SchedulerPoint) {
        let in_val: u32 = sched.get_port(inp).unwrap();
        sched.set_port(out, in_val * 2);
    }
}

pub trait IntoReaction<Params, ReactionT: Reaction> {
    fn reaction(self) -> ReactionT;
}

impl<ReactionT> IntoReaction<(), ReactionT> for F
where
    F: Fn(&ReactionT, &runtime::SchedulerPoint),
{
    fn reaction(self) -> ReactionT {}
}

struct Print;

impl Print {
    pub fn new() -> Self {
        Self
    }

    fn receive(&mut self, sched: &runtime::SchedulerPoint) {
        let value = sched.get_port::<u32>(self.inp);
        event!(tracing::Level::INFO, "Received {:?}", value);
        assert!(matches!(value, Some(2)));
    }
}

impl Reactor for Print {
    type Inputs = (PortKey<u32>);
    type Outputs = ();

    fn build(
        self,
        name: &str,
        env: &mut EnvBuilder,
        parent: Option<ReactorKey>,
    ) -> Result<(ReactorKey, Self::Inputs, Self::Outputs), BuilderError> {
        let builder = env.add_reactor(name, parent, self);
        let i = builder.add_input::<u32>("in2")?;
        let _ = builder
            .add_reaction(Self::receive)
            .with_trigger_port(i)
            .finish(env);
        let key = builder.finish()?;
        Ok((key, i, ()))
    }
}

struct Hierarchy {
    num_prints: usize,
}

impl Reactor for Hierarchy {
    type Inputs = ();
    type Outputs = ();

    fn build(
        self,
        name: &str,
        env: &mut EnvBuilder,
        parent: Option<ReactorKey>,
    ) -> Result<(ReactorKey, Self::Inputs, Self::Outputs), BuilderError> {
        let builder = env.add_reactor(name, parent, self);
        let parent_key = builder.finish()?;

        let (source_key, _, source_out) = Source::new(1).build("source0", env, parent_key)?;
        let (gain_key, gain_in, gain_out) = Gain::new(2).build("gain0", env, parent_key)?;
        env.connect(source_out, gain_in);

        for i in 0..self.num_prints {
            let (print_key, print_in, _) =
                Print::new().build(format!("print{}", i), env, parent_key)?;
            env.connect(gain_out, print_in);
        }

        Ok((parent_key, (), ()))
    }
}

impl Hierarchy {
    fn new(num_prints: usize) -> Self {
        Hierarchy { num_prints }
    }
}

#[test]
fn hierarchy() {
    // install global collector configured based on RUST_LOG env var.
    tracing_subscriber::fmt::init();

    use boomerang::builder::*;
    use boomerang::runtime;

    let mut env_builder = EnvBuilder::new();

    // let gain_container = env_builder.add_reactor_type(
    // "GainContainer",
    // move |mut builder: ReactorTypeBuilderState| {
    // let inp = builder.add_input::<u32>("in").unwrap();
    // let out1 = builder.add_output::<u32>("out1").unwrap();
    // let out2 = builder.add_output::<u32>("out2").unwrap();
    // let gain0 = builder.add_child_instance("gain0", gain);
    //
    // builder.finish()
    // },
    // );

    let (top_key, _, _) = Hierarchy::new(1).build("top", env_builder, None)?;

    let env = env_builder.build(top_key);

    // println!("{:#?}", &env);
    let environment = env.build().unwrap();

    let mut sched = runtime::Scheduler::new(environment.max_level());
    sched.start(environment).unwrap();
}
