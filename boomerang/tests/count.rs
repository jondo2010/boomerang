use std::convert::TryInto;

use boomerang::{ReactorOutputs, runtime};
use boomerang::{builder::*, runtime::SchedulerPoint};

struct Count {
    max_count: u32,
    i: u32,
}

impl Count {
    fn new(max_count: u32) -> Self {
        Self { max_count, i: 0 }
    }
    fn reaction_t(
        &mut self,
        sched: &SchedulerPoint,
        _inputs: &<Self as Reactor>::Inputs,
        outputs: &<Self as Reactor>::Outputs,
        _actions: &<Self as Reactor>::Actions,
    ) {
        self.i += 1;
        sched.set_port(outputs.c, self.i);
        if self.i >= self.max_count {
            sched.shutdown();
        }
    }
}

ReactorOutputs!(CountOutputs, (c, u32));
impl Reactor for Count {
    type Inputs = EmptyPart;
    type Outputs = CountOutputs;
    type Actions = EmptyPart;

    fn build(
        self,
        name: &str,
        env: &mut EnvBuilder,
        parent: Option<runtime::ReactorKey>,
    ) -> Result<(runtime::ReactorKey, Self::Inputs, Self::Outputs), BuilderError> {
        let mut builder = env.add_reactor(name, parent, self);

        let t = builder.add_timer(
            "t",
            runtime::Duration::new(1, 0),
            runtime::Duration::new(0, 0),
        )?;

        let Self::Outputs { c } = builder.outputs;

        builder
            .add_reaction(Self::reaction_t)
            .with_trigger_action(t)
            .with_antidependency(c)
            .finish()?;

        builder.finish()
    }
}

#[test]
fn count() {
    // install global collector configured based on RUST_LOG env var.
    tracing_subscriber::fmt::init();

    let mut env_builder = EnvBuilder::new();

    let (_, _, _) = Count::new(100_000)
        .build("count", &mut env_builder, None)
        .unwrap();

    let env: runtime::Environment = env_builder.try_into().unwrap();
    let mut sched = runtime::Scheduler::new(env.max_level());
    sched.start(env).unwrap();
}
