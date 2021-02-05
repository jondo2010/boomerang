use boomerang::{
    builder::{BuilderError, EmptyPart, EnvBuilder, PortType, Reactor, ReactorPart},
    runtime::{self, PortKey, ReactorKey},
};
use std::convert::TryInto;
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
impl Source {
    pub fn new(value: u32) -> Self {
        Self { value }
    }
    fn reaction_out(
        &mut self,
        sched: &runtime::SchedulerPoint,
        _inputs: &<Self as Reactor>::Inputs,
        outputs: &<Self as Reactor>::Outputs,
        _actions: &<Self as Reactor>::Actions,
    ) {
        sched.set_port(outputs.out, self.value);
        event!(
            tracing::Level::INFO,
            "Sent {:?}",
            sched.get_port::<u32>(outputs.out)
        );
    }
}
#[derive(Clone)]
struct SourceOutputs {
    out: PortKey<u32>,
}
impl ReactorPart for SourceOutputs {
    fn build(env: &mut EnvBuilder, reactor_key: ReactorKey) -> Result<Self, BuilderError> {
        let out = env.add_port("out", PortType::Output, reactor_key)?;
        Ok(Self { out })
    }
}

impl Reactor for Source {
    type Inputs = EmptyPart;
    type Outputs = SourceOutputs;
    type Actions = EmptyPart;

    fn build(
        self,
        name: &str,
        env: &mut EnvBuilder,
        parent: Option<ReactorKey>,
    ) -> Result<(ReactorKey, Self::Inputs, Self::Outputs), BuilderError> {
        let mut builder = env.add_reactor(name, parent, self);
        let t = builder.add_startup_timer("t")?;

        let Self::Outputs { out } = builder.outputs;
        let _ = builder
            .add_reaction(Self::reaction_out)
            .with_trigger_action(t)
            .with_antidependency(out.into())
            .finish();

        builder.finish()
    }
}

struct Gain {
    gain: u32,
}
impl Gain {
    pub fn new(gain: u32) -> Self {
        Self { gain }
    }
    fn reaction_in(
        &mut self,
        sched: &runtime::SchedulerPoint,
        inputs: &<Self as Reactor>::Inputs,
        outputs: &<Self as Reactor>::Outputs,
        _actions: &<Self as Reactor>::Actions,
    ) {
        let in_val: u32 = sched.get_port(inputs.inp).unwrap();
        sched.set_port(outputs.out, in_val * self.gain);
    }
}
#[derive(Clone)]
struct GainInputs {
    inp: PortKey<u32>,
}
impl ReactorPart for GainInputs {
    fn build(env: &mut EnvBuilder, reactor_key: runtime::ReactorKey) -> Result<Self, BuilderError> {
        let inp = env.add_port::<u32>("in", PortType::Input, reactor_key)?;
        Ok(Self { inp })
    }
}
#[derive(Clone)]
struct GainOutputs {
    out: PortKey<u32>,
}
impl ReactorPart for GainOutputs {
    fn build(env: &mut EnvBuilder, reactor_key: runtime::ReactorKey) -> Result<Self, BuilderError> {
        let out = env.add_port::<u32>("out", PortType::Output, reactor_key)?;
        // let out = builder.add_output::<u32>("out")?;
        Ok(Self { out })
    }
}

impl Reactor for Gain {
    type Inputs = GainInputs;
    type Outputs = GainOutputs;
    type Actions = EmptyPart;
    fn build(
        self,
        name: &str,
        env: &mut EnvBuilder,
        parent: Option<ReactorKey>,
    ) -> Result<(ReactorKey, Self::Inputs, Self::Outputs), BuilderError> {
        let mut builder = env.add_reactor(name, parent, self);

        let Self::Inputs { inp } = builder.inputs;
        let Self::Outputs { out } = builder.outputs;
        let _ = builder
            .add_reaction(Self::reaction_in)
            .with_trigger_port(inp.into())
            .with_antidependency(out.into())
            .finish();

        builder.finish()
    }
}

struct Print;
impl Print {
    pub fn new() -> Self {
        Self
    }
    fn reaction_in(
        &mut self,
        sched: &runtime::SchedulerPoint,
        inputs: &<Self as Reactor>::Inputs,
        _outputs: &<Self as Reactor>::Outputs,
        _actions: &<Self as Reactor>::Actions,
    ) {
        let value = sched.get_port::<u32>(inputs.inp);
        event!(tracing::Level::INFO, "Received {:?}", value);
        assert!(matches!(value, Some(2)));
    }
}
#[derive(Clone)]
struct PrintInputs {
    inp: PortKey<u32>,
}

impl ReactorPart for PrintInputs {
    fn build(env: &mut EnvBuilder, reactor_key: runtime::ReactorKey) -> Result<Self, BuilderError> {
        let inp = env.add_port::<u32>("in2", PortType::Input, reactor_key)?;
        Ok(Self { inp })
    }
}

impl Reactor for Print {
    type Inputs = PrintInputs;
    type Outputs = EmptyPart;
    type Actions = EmptyPart;

    fn build(
        self,
        name: &str,
        env: &mut EnvBuilder,
        parent: Option<ReactorKey>,
    ) -> Result<(ReactorKey, Self::Inputs, Self::Outputs), BuilderError> {
        let mut builder = env.add_reactor(name, parent, self);
        let Self::Inputs { inp } = builder.inputs;
        let _ = builder
            .add_reaction(Self::reaction_in)
            .with_trigger_port(inp.into())
            .finish();
        builder.finish()
    }
}

struct Hierarchy {
    num_prints: usize,
}

impl Reactor for Hierarchy {
    type Inputs = EmptyPart;
    type Outputs = EmptyPart;
    type Actions = EmptyPart;

    fn build(
        self,
        name: &str,
        env: &mut EnvBuilder,
        parent: Option<ReactorKey>,
    ) -> Result<(ReactorKey, Self::Inputs, Self::Outputs), BuilderError> {
        let num_prints = self.num_prints;
        let builder = env.add_reactor(name, parent, self);
        let (parent_key, _, _) = builder.finish()?;

        let (_source_key, _, source_out) =
            Source::new(1).build("source0", env, Some(parent_key))?;
        let (_gain_key, gain_in, gain_out) = Gain::new(2).build("gain0", env, Some(parent_key))?;
        env.bind_port(source_out.out, gain_in.inp)?;

        for i in 0..num_prints {
            let (_print_key, print_in, _) =
                Print::new().build(&format!("print{}", i), env, Some(parent_key))?;
            env.bind_port(gain_out.out, print_in.inp)?;
        }

        Ok((parent_key, EmptyPart, EmptyPart))
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

    let (_, _, _) = Hierarchy::new(2)
        .build("top", &mut env_builder, None)
        .unwrap();

    for (a, b) in env_builder.reaction_dependency_edges() {
        println!(
            "{}->{}",
            env_builder.reaction_fqn(a).unwrap(),
            env_builder.reaction_fqn(b).unwrap()
        );
    }

    let env: runtime::Environment = env_builder.try_into().unwrap();
    //println!("{:#?}", &env);

    let mut sched = runtime::Scheduler::new(env.max_level());
    sched.start(env).unwrap();
}
