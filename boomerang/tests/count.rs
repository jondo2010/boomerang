use std::convert::TryInto;

use boomerang::{builder::*, runtime::SchedulerPoint};
use boomerang::{runtime, ReactorOutputs};

struct Count {
    max_count: u32,
    i: u32,
}

impl Count {
    fn new(max_count: u32) -> Self {
        Self { max_count, i: 0 }
    }
    fn reaction_t<S: SchedulerPoint>(
        &mut self,
        sched: &S,
        _inputs: &EmptyPart,
        outputs: &CountOutputs,
        _actions: &EmptyPart,
    ) {
        self.i += 1;
        sched.get_port_with_mut(outputs.c, |value, _is_set| {
            *value = self.i;
            true
        });
        if self.i >= self.max_count {
            sched.shutdown();
        }
    }
}

ReactorOutputs!(Count, CountOutputs, (c, u32));
impl<'a, S: SchedulerPoint> Reactor<S> for Count {
    type Inputs = EmptyPart;
    type Outputs = CountOutputs;
    type Actions = EmptyPart;

    fn build(
        self,
        name: &str,
        env: &mut EnvBuilder<S>,
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
            .add_reaction("reaction_t", Self::reaction_t)
            .with_trigger_action(t)
            .with_antidependency(c.into())
            .finish()?;

        builder.finish()
    }

    fn build_parts(
        &self,
        env: &mut EnvBuilder<S>,
        reactor_key: runtime::ReactorKey,
    ) -> Result<(Self::Inputs, Self::Outputs, Self::Actions), BuilderError> {
        Ok((EmptyPart, CountOutputs::build(env, reactor_key)?, EmptyPart))
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

    let gv = graphviz::build(&env_builder).unwrap();
    let mut f = std::fs::File::create(format!("{}.dot", module_path!())).unwrap();
    std::io::Write::write_all(&mut f, gv.as_bytes()).unwrap();

    let gv = graphviz::build_reaction_graph(&env_builder).unwrap();
    let mut f = std::fs::File::create(format!("{}_levels.dot", module_path!())).unwrap();
    std::io::Write::write_all(&mut f, gv.as_bytes()).unwrap();

    let env: runtime::Env<_> = env_builder.try_into().unwrap();
    let mut sched = runtime::Scheduler::new(env);
    sched.start().unwrap();
}
