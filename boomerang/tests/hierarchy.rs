use std::convert::TryInto;

use boomerang::{
    builder::{BuilderError, EnvBuilder, Reactor},
    runtime::{self, PortKey, ReactorKey},
};
use slotmap::Key;

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
    type Outputs = PortKey<u32>;

    fn build(
        self,
        name: &str,
        env: &mut EnvBuilder,
        parent: Option<ReactorKey>,
    ) -> Result<(ReactorKey, Self::Inputs, Self::Outputs), BuilderError> {
        let mut builder = env.add_reactor(name, parent, self);
        let out = builder.add_output::<u32>("out")?;
        let t = builder.add_startup_timer("t")?;

        let _r0 = builder
            .add_reaction(Self::r0)
            .with_trigger_action(t)
            .with_antidependency(out.data().into())
            .finish();

        let key = builder.finish()?;
        Ok((key, (), (out)))
    }
}

impl Source {
    pub fn new(value: u32) -> Self {
        Self { value }
    }

    fn r0(&mut self, _sched: &runtime::SchedulerPoint) {
        // sched.set_port(self.out, 1u32);
        // event!(
        // tracing::Level::INFO,
        // "Sent {:?}",
        // sched.get_port::<u32>(self.out)
        // );
    }
}

struct Gain {
    gain: u32,
}

impl Reactor for Gain {
    type Inputs = PortKey<u32>;
    type Outputs = PortKey<u32>;
    fn build(
        self,
        name: &str,
        env: &mut EnvBuilder,
        parent: Option<ReactorKey>,
    ) -> Result<(ReactorKey, Self::Inputs, Self::Outputs), BuilderError> {
        let mut builder = env.add_reactor(name, parent, self);
        let inp = builder.add_input::<u32>("in")?;
        let out = builder.add_output::<u32>("out")?;

        let _ = builder
            .add_reaction(Self::r0)
            .with_trigger_port(inp.data().into())
            .with_antidependency(out.data().into())
            .finish();

        let key = builder.finish()?;
        Ok((key, inp, out))
    }
}

impl Gain {
    pub fn new(gain: u32) -> Self {
        Self { gain }
    }
    fn r0(&mut self, _sched: &runtime::SchedulerPoint) {
        // let in_val: u32 = sched.get_port(inp).unwrap();
        // sched.set_port(out, in_val * 2);
    }
}

struct Print;

impl Print {
    pub fn new() -> Self {
        Self
    }

    fn receive(&mut self, _sched: &runtime::SchedulerPoint) {
        // let value = sched.get_port::<u32>(self.inp);
        // event!(tracing::Level::INFO, "Received {:?}", value);
        // assert!(matches!(value, Some(2)));
    }
}

impl Reactor for Print {
    type Inputs = PortKey<u32>;
    type Outputs = ();

    fn build(
        self,
        name: &str,
        env: &mut EnvBuilder,
        parent: Option<ReactorKey>,
    ) -> Result<(ReactorKey, Self::Inputs, Self::Outputs), BuilderError> {
        let mut builder = env.add_reactor(name, parent, self);
        let i = builder.add_input::<u32>("in2")?;
        let _ = builder
            .add_reaction(Self::receive)
            .with_trigger_port(i.data().into())
            .finish();
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
        let num_prints = self.num_prints;
        let builder = env.add_reactor(name, parent, self);
        let parent_key = builder.finish()?;

        let (_source_key, _, source_out) =
            Source::new(1).build("source0", env, Some(parent_key))?;
        let (_gain_key, gain_in, gain_out) = Gain::new(2).build("gain0", env, Some(parent_key))?;
        env.bind_port(source_out, gain_in)?;

        for i in 0..num_prints {
            let (_print_key, print_in, _) =
                Print::new().build(&format!("print{}", i), env, Some(parent_key))?;
            env.bind_port(gain_out, print_in)?;
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

    let (_, _, _) = Hierarchy::new(2).build("top", &mut env_builder, None).unwrap();

    for (a, b) in env_builder.reaction_dependency_edges() {
        println!(
            "{}->{}",
            env_builder.reaction_fqn(a).unwrap(),
            env_builder.reaction_fqn(b).unwrap()
        );
    }

    let env: runtime::Environment = env_builder.try_into().unwrap();
    println!("{}", &env);

    // let mut sched = runtime::Scheduler::new(environment.max_level());
    // sched.start(environment).unwrap();
}
