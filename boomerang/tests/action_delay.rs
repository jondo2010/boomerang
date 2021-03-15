use boomerang::{
    builder::{EmptyPart, EnvBuilder, Reactor},
    runtime::{self, Duration, SchedulerPoint},
    ReactorActions, ReactorInputs, ReactorOutputs,
};
use std::convert::TryInto;

// Test logical action with delay.
// reactor GeneratedDelay {
//     input y_in:int;
//     output y_out:int;
//     state y_state:int(0);
//     logical action act(100 msec):void;
//     reaction(y_in) -> act {=
//         y_state = *y_in.get();
//         act.schedule();
//     =}
//
//     reaction(act) -> y_out {=
//         y_out.set(y_state);
//     =}
// }
//
// reactor Source {
//     output out:int;
//     reaction(startup) -> out {=
//         out.set(1);
//     =}
// }
//
// reactor Sink {
// 	input in:int;
// 	reaction(in) {=
//         auto elapsed_logical = get_elapsed_logical_time();
//         auto logical = get_logical_time();
//         auto physical = get_physical_time();
//         std::cout << "logical time: " << logical << '\n';
//         std::cout << "physical time: " << physical << '\n';
//         std::cout << "elapsed logical time: " << elapsed_logical << '\n';
//         if (elapsed_logical != 100ms) {
//             std::cerr << "ERROR: Expected 100 msecs but got " << elapsed_logical << '\n';
//             exit(1);
//         } else {
//             std::cout << "SUCCESS. Elapsed logical time is 100 msec.\n";
//         }
// 	=}
// }
//
// main reactor ActionDelay {
//     source = new Source();
//     sink = new Sink();
//     sink2 = new Sink();
//     sink3 = new Sink();
//
//     g = new GeneratedDelay();
//
//     source.out -> g.y_in;
//     source.out -> sink2.in;
//
//     g.y_out -> sink.in;
//     g.y_out -> sink3.in;
// }

struct GeneratedDelay {
    y_state: u32,
}

impl GeneratedDelay {
    fn new() -> Self {
        Self { y_state: 0 }
    }

    // y_in -> act
    fn reaction_y_in<S: SchedulerPoint>(
        &mut self,
        sched: &S,
        inputs: &GeneratedDelayInputs,
        _outputs: &GeneratedDelayOutputs,
        actions: &GeneratedDelayActions,
    ) {
        sched.get_port_with(inputs.y_in, |value, is_set| {
            if is_set {
                self.y_state = *value;
            }
        });
        sched.schedule_action(actions.act, (), None);
    }

    // act -> y_out
    fn reaction_act<S: SchedulerPoint>(
        &mut self,
        sched: &S,
        _inputs: &<Self as Reactor<S>>::Inputs,
        outputs: &<Self as Reactor<S>>::Outputs,
        _actions: &<Self as Reactor<S>>::Actions,
    ) {
        sched.get_port_with_mut(outputs.y_out, |value, _is_set| {
            *value = self.y_state;
            true
        });
    }
}

ReactorInputs!(GeneratedDelay, GeneratedDelayInputs, (y_in, u32));
ReactorOutputs!(GeneratedDelay, GeneratedDelayOutputs, (y_out, u32));
ReactorActions!(
    GeneratedDelay,
    GeneratedDelayActions,
    (act, (), Some(Duration::from_millis(100)))
);

impl<'a, S: SchedulerPoint> Reactor<S> for GeneratedDelay {
    type Inputs = GeneratedDelayInputs;
    type Outputs = GeneratedDelayOutputs;
    type Actions = GeneratedDelayActions;

    fn build(
        self,
        name: &str,
        env: &mut boomerang::builder::EnvBuilder<S>,
        parent: Option<boomerang::runtime::ReactorKey>,
    ) -> Result<
        (boomerang::runtime::ReactorKey, Self::Inputs, Self::Outputs),
        boomerang::builder::BuilderError,
    > {
        let mut builder = env.add_reactor(name, parent, self);
        let Self::Inputs { y_in } = builder.inputs;
        let Self::Outputs { y_out } = builder.outputs;
        let Self::Actions { act } = builder.actions;
        let _ = builder
            .add_reaction("reaction_y_in", Self::reaction_y_in)
            .with_trigger_port(y_in.into())
            .with_scheduable_action(act.into())
            .finish()?;
        let _ = builder
            .add_reaction("reaction_act", Self::reaction_act)
            .with_trigger_action(act.into())
            .with_antidependency(y_out.into())
            .finish()?;
        builder.finish()
    }

    fn build_parts(
        &self,
        env: &mut EnvBuilder<S>,
        reactor_key: runtime::ReactorKey,
    ) -> Result<(Self::Inputs, Self::Outputs, Self::Actions), boomerang::builder::BuilderError>
    {
        Ok((
            Self::Inputs::build(env, reactor_key)?,
            Self::Outputs::build(env, reactor_key)?,
            Self::Actions::build(env, reactor_key)?,
        ))
    }
}

struct Source;
impl Source {
    fn reaction_startup<S: SchedulerPoint>(
        &mut self,
        sched: &S,
        _inputs: &EmptyPart,
        outputs: &SourceOutputs,
        _actions: &EmptyPart,
    ) {
        sched.get_port_with_mut(outputs.out, |value, _is_set| {
            *value = 1u32;
            true
        });
    }
}
ReactorOutputs!(Source, SourceOutputs, (out, u32));
impl<'a, S: SchedulerPoint> Reactor<S> for Source {
    type Inputs = EmptyPart;
    type Outputs = SourceOutputs;
    type Actions = EmptyPart;

    fn build(
        self,
        name: &str,
        env: &mut boomerang::builder::EnvBuilder<S>,
        parent: Option<boomerang::runtime::ReactorKey>,
    ) -> Result<
        (boomerang::runtime::ReactorKey, Self::Inputs, Self::Outputs),
        boomerang::builder::BuilderError,
    > {
        let mut builder = env.add_reactor(name, parent, self);
        let startup = builder.add_startup_action("startup")?;
        let Self::Outputs { out } = builder.outputs;
        let _ = builder
            .add_reaction("reaction_startup", Self::reaction_startup)
            .with_trigger_action(startup)
            .with_antidependency(out.into())
            .finish()?;
        builder.finish()
    }

    fn build_parts(
        &self,
        env: &mut EnvBuilder<S>,
        reactor_key: runtime::ReactorKey,
    ) -> Result<(Self::Inputs, Self::Outputs, Self::Actions), boomerang::builder::BuilderError>
    {
        Ok((
            EmptyPart,
            Self::Outputs::build(env, reactor_key)?,
            EmptyPart,
        ))
    }
}

struct Sink;
ReactorInputs!(Sink, SinkInputs, (inp, u32));
impl Sink {
    fn reaction_in<S: SchedulerPoint>(
        &mut self,
        sched: &S,
        _inputs: &SinkInputs,
        _outputs: &EmptyPart,
        _actions: &EmptyPart,
    ) {
        let elapsed_logical = sched.get_elapsed_logical_time();
        let logical = sched.get_logical_time();
        let physical = sched.get_physical_time();
        println!("logical time: {:?}", logical);
        println!("physical time: {:?}", physical);
        println!("elapsed logical time: {:?}", elapsed_logical);
        assert!(
            elapsed_logical == Duration::from_millis(100),
            "ERROR: Expected 100 msecs but got {:?}",
            elapsed_logical
        );
        println!("SUCCESS. Elapsed logical time is 100 msec.");
    }
}
impl<'a, S: SchedulerPoint> Reactor<S> for Sink {
    type Inputs = SinkInputs;
    type Outputs = EmptyPart;
    type Actions = EmptyPart;
    fn build(
        self,
        name: &str,
        env: &mut boomerang::builder::EnvBuilder<S>,
        parent: Option<boomerang::runtime::ReactorKey>,
    ) -> Result<
        (boomerang::runtime::ReactorKey, Self::Inputs, Self::Outputs),
        boomerang::builder::BuilderError,
    > {
        let mut builder = env.add_reactor(name, parent, self);
        let Self::Inputs { inp } = builder.inputs;
        let _ = builder
            .add_reaction("reaction_in", Self::reaction_in)
            .with_trigger_port(inp.into())
            .finish()?;
        builder.finish()
    }

    fn build_parts(
        &self,
        env: &mut EnvBuilder<S>,
        reactor_key: runtime::ReactorKey,
    ) -> Result<(Self::Inputs, Self::Outputs, Self::Actions), boomerang::builder::BuilderError>
    {
        Ok((SinkInputs::build(env, reactor_key)?, EmptyPart, EmptyPart))
    }
}

#[derive(Default)]
struct ActionDelay;

impl<'a, S: SchedulerPoint> Reactor<S> for ActionDelay {
    type Inputs = EmptyPart;
    type Outputs = EmptyPart;
    type Actions = EmptyPart;

    fn build(
        self,
        name: &str,
        env: &mut EnvBuilder<S>,
        parent: Option<runtime::ReactorKey>,
    ) -> Result<(runtime::ReactorKey, Self::Inputs, Self::Outputs), boomerang::builder::BuilderError>
    {
        let (parent_key, _, _) = env.add_reactor(name, parent, self).finish()?;
        let (_, _, source_outputs) = Source.build("source", env, Some(parent_key))?;
        let (_, sink0_inputs, _) = Sink.build("sink0", env, Some(parent_key))?;
        let (_, _sink1_inputs, _) = Sink.build("sink1", env, Some(parent_key))?;
        let (_, sink2_inputs, _) = Sink.build("sink2", env, Some(parent_key))?;
        let (_, g_inputs, g_outputs) = GeneratedDelay::new().build("g", env, Some(parent_key))?;
        env.bind_port(source_outputs.out.into(), g_inputs.y_in.into()).unwrap();
        // env.bind_port(source_outputs.out, sink1_inputs.inp).unwrap();
        env.bind_port(g_outputs.y_out.into(), sink0_inputs.inp.into()).unwrap();
        env.bind_port(g_outputs.y_out.into(), sink2_inputs.inp.into()).unwrap();
        Ok((parent_key, EmptyPart::default(), EmptyPart::default()))
    }

    fn build_parts(
        &self,
        _env: &mut EnvBuilder<S>,
        _reactor_key: runtime::ReactorKey,
    ) -> Result<(Self::Inputs, Self::Outputs, Self::Actions), boomerang::builder::BuilderError>
    {
        Ok((EmptyPart, EmptyPart, EmptyPart))
    }
}

#[test]
fn action_delay() {
    // install global collector configured based on RUST_LOG env var.
    tracing_subscriber::fmt::init();

    let mut env_builder = EnvBuilder::new();
    let _ = ActionDelay
        .build("action_delay", &mut env_builder, None)
        .unwrap();

    let env: runtime::Env<_> = env_builder.try_into().unwrap();
    let mut sched = runtime::Scheduler::new(env);
    sched.start().unwrap();
}
