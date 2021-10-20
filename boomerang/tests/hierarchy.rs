use boomerang::{
    builder::{BuilderError, EmptyPart, EnvBuilder, Reactor},
    runtime::{self, ReactorKey},
    ReactorInputs, ReactorOutputs,
};
use runtime::SchedulerPoint;
use std::{convert::TryInto, io::stdout};
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
    fn reaction_out<S: SchedulerPoint>(
        &mut self,
        sched: &S,
        _inputs: &<Self as Reactor<S>>::Inputs,
        outputs: &<Self as Reactor<S>>::Outputs,
        _actions: &<Self as Reactor<S>>::Actions,
    ) {
        sched.get_port_with_mut(outputs.out, |value, _is_set| {
            *value = self.value;
            true
        });

        event!(tracing::Level::INFO, "Sent {:?}", self.value,);
    }
}

ReactorOutputs!(Source, SourceOutputs, (out, u32));

impl<S: SchedulerPoint> Reactor<S> for Source {
    type Inputs = EmptyPart;
    type Outputs = SourceOutputs;
    type Actions = EmptyPart;

    fn build(
        self,
        name: &str,
        env: &mut EnvBuilder<S>,
        parent: Option<ReactorKey>,
    ) -> Result<(ReactorKey, Self::Inputs, Self::Outputs), BuilderError> {
        let mut builder = env.add_reactor(name, parent, self);
        let t = builder.add_startup_action("t")?;

        let Self::Outputs { out } = builder.outputs;
        let _ = builder
            .add_reaction("reaction_out", Self::reaction_out)
            .with_trigger_action(t)
            .with_antidependency(out.into())
            .finish();

        builder.finish()
    }

    fn build_parts(
        &self,
        env: &mut EnvBuilder<S>,
        reactor_key: ReactorKey,
    ) -> Result<(Self::Inputs, Self::Outputs, Self::Actions), BuilderError> {
        Ok((
            EmptyPart,
            SourceOutputs::build(env, reactor_key)?,
            EmptyPart,
        ))
    }
}

struct Gain {
    gain: u32,
}
impl Gain {
    pub fn new(gain: u32) -> Self {
        Self { gain }
    }
    fn reaction_in<S: SchedulerPoint>(
        &mut self,
        sched: &S,
        inputs: &<Self as Reactor<S>>::Inputs,
        outputs: &<Self as Reactor<S>>::Outputs,
        _actions: &<Self as Reactor<S>>::Actions,
    ) {
        sched.get_port_with(inputs.inp, |inp: &u32, is_set| {
            if is_set {
                sched.get_port_with_mut(outputs.out, |out: &mut u32, _is_set| {
                    *out = inp * self.gain;
                    true
                });
            }
        });
    }
}

ReactorInputs!(Gain, GainInputs, (inp, u32));
ReactorOutputs!(Gain, GainOutputs, (out, u32));

impl<S: SchedulerPoint> Reactor<S> for Gain {
    type Inputs = GainInputs;
    type Outputs = GainOutputs;
    type Actions = EmptyPart;
    fn build(
        self,
        name: &str,
        env: &mut EnvBuilder<S>,
        parent: Option<ReactorKey>,
    ) -> Result<(ReactorKey, Self::Inputs, Self::Outputs), BuilderError> {
        let mut builder = env.add_reactor(name, parent, self);

        let Self::Inputs { inp } = builder.inputs;
        let Self::Outputs { out } = builder.outputs;
        let _ = builder
            .add_reaction("reaction_in", Self::reaction_in)
            .with_trigger_port(inp.into())
            .with_antidependency(out.into())
            .finish();

        builder.finish()
    }

    fn build_parts(
        &self,
        env: &mut EnvBuilder<S>,
        reactor_key: ReactorKey,
    ) -> Result<(Self::Inputs, Self::Outputs, Self::Actions), BuilderError> {
        Ok((
            GainInputs::build(env, reactor_key)?,
            GainOutputs::build(env, reactor_key)?,
            EmptyPart,
        ))
    }
}

struct Print;
impl Print {
    pub fn new() -> Self {
        Self
    }
    fn reaction_in<S: SchedulerPoint>(
        &mut self,
        sched: &S,
        inputs: &<Self as Reactor<S>>::Inputs,
        _outputs: &<Self as Reactor<S>>::Outputs,
        _actions: &<Self as Reactor<S>>::Actions,
    ) {
        sched.get_port_with(inputs.inp, |value, is_set| {
            event!(tracing::Level::INFO, "Received {:?}", value);
            assert!(is_set);
            assert!(matches!(value, 2u32));
        });
    }
}

ReactorInputs!(Print, PrintInputs, (inp, u32));
impl<S: SchedulerPoint> Reactor<S> for Print {
    type Inputs = PrintInputs;
    type Outputs = EmptyPart;
    type Actions = EmptyPart;

    fn build(
        self,
        name: &str,
        env: &mut EnvBuilder<S>,
        parent: Option<ReactorKey>,
    ) -> Result<(ReactorKey, Self::Inputs, Self::Outputs), BuilderError> {
        let mut builder = env.add_reactor(name, parent, self);
        let Self::Inputs { inp } = builder.inputs;
        let _ = builder
            .add_reaction("reaction_in", Self::reaction_in)
            .with_trigger_port(inp.into())
            .finish();
        builder.finish()
    }

    fn build_parts(
        &self,
        env: &mut EnvBuilder<S>,
        reactor_key: ReactorKey,
    ) -> Result<(Self::Inputs, Self::Outputs, Self::Actions), BuilderError> {
        Ok((PrintInputs::build(env, reactor_key)?, EmptyPart, EmptyPart))
    }
}

struct GainContainer;
impl GainContainer {
    pub fn new() -> Self {
        Self
    }
}

ReactorInputs!(GainContainer, GainContainerInputs, (inp, u32));
ReactorOutputs!(
    GainContainer,
    GainContainerOutputs,
    (out1, u32),
    (out2, u32)
);
impl<S: SchedulerPoint> Reactor<S> for GainContainer {
    type Inputs = GainContainerInputs;
    type Outputs = GainContainerOutputs;
    type Actions = EmptyPart;

    fn build(
        self,
        name: &str,
        env: &mut EnvBuilder<S>,
        parent: Option<ReactorKey>,
    ) -> Result<(ReactorKey, Self::Inputs, Self::Outputs), BuilderError> {
        let builder = env.add_reactor(name, parent, self);
        let (parent_key, inputs, outputs) = builder.finish()?;
        let (_gain_key, gain_in, gain_out) = Gain::new(2).build("gain", env, Some(parent_key))?;
        env.bind_port(inputs.inp.into(), gain_in.inp.into())?;
        env.bind_port(gain_out.out.into(), outputs.out1.into())?;
        env.bind_port(gain_out.out.into(), outputs.out2.into())?;
        Ok((parent_key, inputs, outputs))
    }

    fn build_parts(
        &self,
        env: &mut EnvBuilder<S>,
        reactor_key: ReactorKey,
    ) -> Result<(Self::Inputs, Self::Outputs, Self::Actions), BuilderError> {
        Ok((
            GainContainerInputs::build(env, reactor_key)?,
            GainContainerOutputs::build(env, reactor_key)?,
            EmptyPart,
        ))
    }
}

struct Hierarchy {
    num_prints: usize,
}

impl<S: SchedulerPoint> Reactor<S> for Hierarchy {
    type Inputs = EmptyPart;
    type Outputs = EmptyPart;
    type Actions = EmptyPart;

    fn build(
        self,
        name: &str,
        env: &mut EnvBuilder<S>,
        parent: Option<ReactorKey>,
    ) -> Result<(ReactorKey, Self::Inputs, Self::Outputs), BuilderError> {
        let num_prints = self.num_prints;
        let builder = env.add_reactor(name, parent, self);
        let (parent_key, _, _) = builder.finish()?;

        let (_source_key, _, source_out) =
            Source::new(1).build("source0", env, Some(parent_key))?;
        let (_, container_in, container_out) =
            GainContainer::new().build("container", env, Some(parent_key))?;
        env.bind_port(source_out.out.into(), container_in.inp.into())?;

        for i in 0..num_prints {
            let (_print_key, print_in, _) =
                Print::new().build(&format!("print{}", i), env, Some(parent_key))?;
            if i % 2 == 0 {
                env.bind_port(container_out.out1.into(), print_in.inp.into())?;
            } else {
                env.bind_port(container_out.out2.into(), print_in.inp.into())?;
            }
        }

        Ok((parent_key, EmptyPart, EmptyPart))
    }

    fn build_parts(
        &self,
        _env: &mut EnvBuilder<S>,
        _reactor_key: ReactorKey,
    ) -> Result<(Self::Inputs, Self::Outputs, Self::Actions), BuilderError> {
        Ok((EmptyPart, EmptyPart, EmptyPart))
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

    let (_, _, _) = Hierarchy::new(4)
        .build("top", &mut env_builder, None)
        .unwrap();

    // boomerang::builder::graphviz::reaction_graph::render_to(&env_builder, &mut stdout());
    let gv = graphviz::build(&env_builder).unwrap();
    let mut f = std::fs::File::create("out.dot").unwrap();
    std::io::Write::write_all(&mut f, gv.as_bytes()).unwrap();

    let env: runtime::Env<_> = env_builder.try_into().unwrap();
    let mut sched = runtime::Scheduler::new(env);
    sched.start().unwrap();
}
