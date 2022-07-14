use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::convert::TryInto;

use boomerang::{builder::*, runtime, ReactorActions, ReactorInputs, ReactorOutputs};

struct Ping {
    pings_left: u32,
}

ReactorInputs!(Ping, PingInputs, (receive, u32));
ReactorOutputs!(Ping, PingOutputs, (send, u32));
ReactorActions!(Ping, PingActions, (serve, (), None));

impl Ping {
    fn new(count: u32) -> Self {
        Self { pings_left: count }
    }
    fn reaction_startup_serve<S: runtime::SchedulerPoint>(
        &mut self,
        sched: &S,
        _inputs: &PingInputs,
        outputs: &PingOutputs,
        _actions: &PingActions,
    ) {
        sched.get_port_with_mut(outputs.send, |send, _is_set| {
            *send = self.pings_left;
            self.pings_left -= 1;
            true
        })
    }

    fn reaction_receive<S: runtime::SchedulerPoint>(
        &mut self,
        sched: &S,
        _inputs: &PingInputs,
        _outputs: &PingOutputs,
        actions: &PingActions,
    ) {
        if self.pings_left > 0 {
            sched.schedule_action(actions.serve, (), None);
        } else {
            sched.shutdown();
        }
    }
}

impl<S: runtime::SchedulerPoint> Reactor<S> for Ping {
    type Inputs = PingInputs;
    type Outputs = PingOutputs;
    type Actions = PingActions;

    fn build_parts<'b>(
        &'b self,
        env: &'b mut EnvBuilder<S>,
        reactor_key: runtime::ReactorKey,
    ) -> Result<(Self::Inputs, Self::Outputs, Self::Actions), BuilderError> {
        Ok((
            PingInputs::build(env, reactor_key)?,
            PingOutputs::build(env, reactor_key)?,
            PingActions::build(env, reactor_key)?,
        ))
    }

    fn build(
        self,
        name: &str,
        env: &mut EnvBuilder<S>,
        parent: Option<runtime::ReactorKey>,
    ) -> Result<(runtime::ReactorKey, Self::Inputs, Self::Outputs), BuilderError> {
        let mut builder = env.add_reactor(name, parent, self);
        let PingInputs { receive } = builder.inputs;
        let PingOutputs { send } = builder.outputs;
        let PingActions { serve } = builder.actions;
        let startup = builder.add_startup_action("startup")?;
        let _ = builder
            .add_reaction("receive", Self::reaction_receive)
            .with_trigger_port(receive)
            .with_scheduable_action(serve)
            .finish();
        let _ = builder
            .add_reaction("startup_serve", Self::reaction_startup_serve)
            .with_trigger_action(startup)
            .with_trigger_action(serve)
            .with_antidependency(send)
            .finish();
        builder.finish()
    }
}

struct Pong {
    expected: u32,
    count: u32,
}
ReactorInputs!(Pong, PongInputs, (receive, u32));
ReactorOutputs!(Pong, PongOutputs, (send, u32));

impl Pong {
    fn new(expected: u32) -> Self {
        Self { expected, count: 0 }
    }
    fn reaction_receive<S: runtime::SchedulerPoint>(
        &mut self,
        sched: &S,
        inputs: &PongInputs,
        outputs: &PongOutputs,
        _actions: &EmptyPart,
    ) {
        sched.get_port_with(inputs.receive, |receive: &u32, is_set| {
            if is_set {
                self.count += 1;
                sched.get_port_with_mut(outputs.send, |send, _is_set| {
                    *send = *receive;
                    true
                });
            }
        });
    }
    fn reaction_shutdown<S: runtime::SchedulerPoint>(
        &mut self,
        _sched: &S,
        _inputs: &PongInputs,
        _outputs: &PongOutputs,
        _actions: &EmptyPart,
    ) {
        assert_eq!(self.count, self.expected);
    }
}

impl<S: runtime::SchedulerPoint> Reactor<S> for Pong {
    type Inputs = PongInputs;
    type Outputs = PongOutputs;
    type Actions = EmptyPart;

    fn build_parts<'b>(
        &'b self,
        env: &'b mut EnvBuilder<S>,
        reactor_key: runtime::ReactorKey,
    ) -> Result<(Self::Inputs, Self::Outputs, Self::Actions), BuilderError> {
        Ok((
            PongInputs::build(env, reactor_key)?,
            PongOutputs::build(env, reactor_key)?,
            EmptyPart,
        ))
    }

    fn build(
        self,
        name: &str,
        env: &mut EnvBuilder<S>,
        parent: Option<runtime::ReactorKey>,
    ) -> Result<(runtime::ReactorKey, Self::Inputs, Self::Outputs), BuilderError> {
        let mut builder = env.add_reactor(name, parent, self);
        let PongInputs { receive } = builder.inputs;
        let PongOutputs { send } = builder.outputs;

        let send_internal = builder.add_internal_port::<()>(name, PortType::Output)?;

        let _ = builder
            .add_reaction("receive", Self::reaction_receive)
            .with_trigger_port(receive)
            .with_antidependency(send)
            .with_antidependency(send_internal)
            .finish();
        let shutdown = builder.add_shutdown_action("shutdown")?;
        let _ = builder
            .add_reaction("shutdown", Self::reaction_shutdown)
            .with_trigger_action(shutdown)
            .finish();

        builder.finish()
    }
}

struct SavinaPong {
    count: u32,
}
impl<S: runtime::SchedulerPoint> Reactor<S> for SavinaPong {
    type Inputs = EmptyPart;
    type Outputs = EmptyPart;
    type Actions = EmptyPart;

    fn build_parts<'b>(
        &'b self,
        _env: &'b mut EnvBuilder<S>,
        _reactor_key: runtime::ReactorKey,
    ) -> Result<(Self::Inputs, Self::Outputs, Self::Actions), BuilderError> {
        Ok((EmptyPart, EmptyPart, EmptyPart))
    }

    fn build(
        self,
        name: &str,
        env: &mut EnvBuilder<S>,
        parent: Option<runtime::ReactorKey>,
    ) -> Result<(runtime::ReactorKey, Self::Inputs, Self::Outputs), BuilderError> {
        let count = self.count;
        let builder = env.add_reactor(name, parent, self);

        let (parent_key, _, _) = builder.finish()?;

        let (_ping_key, ping_inputs, ping_outputs) =
            Ping::new(count).build("ping", env, Some(parent_key))?;

        let (_pong_key, pong_inputs, pong_outputs) =
            Pong::new(count).build("pong", env, Some(parent_key))?;
        env.bind_port(ping_outputs.send, pong_inputs.receive)?;
        env.bind_port(pong_outputs.send, ping_inputs.receive)?;

        Ok((parent_key, EmptyPart, EmptyPart))
    }
}

fn savina_pong(count: u32) {
    use boomerang::builder::*;

    let mut env_builder = EnvBuilder::new();
    let (_, _, _) = SavinaPong { count }
        .build("savina_pong", &mut env_builder, None)
        .unwrap();

    let env: runtime::Env<_> = env_builder.try_into().unwrap();
    let mut sched = runtime::Scheduler::new(env);
    sched.start().unwrap();
}

fn criterion_benchmark(c: &mut Criterion) {
    c.bench_function("savina_pong", |bencher| {
        bencher.iter(|| savina_pong(black_box(100_000)))
    });
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
