// reactor Count {
//    output c:int;
//    timer t(0, 1 sec);
//    state i:int(0);
//    reaction(t) -> c {=
//        i++;
//        c.set(i);
//    =}
//}

use std::sync::{Arc, RwLock};

use boomerang::runtime;
use boomerang::{
    builder::*,
    runtime::{ActionIndex, PortIndex, SchedulerPoint},
};

struct Count {
    c: PortIndex,
    t: ActionIndex,
    i: u32,
}

impl Count {
    fn build(builder: &mut ReactorTypeBuilderState) -> Arc<RwLock<Self>> {
        let c = builder.add_output::<u32>("c").unwrap();
        let t = builder.add_timer(
            "t",
            runtime::Duration::new(1, 0),
            runtime::Duration::new(0, 0),
        );

        let this = Arc::new(RwLock::new(Self { c, t, i: 0 }));
        let this2 = this.clone();

        builder
            .add_reaction("r0", move |sched| {
                this2.write().unwrap().r0(sched);
            })
            .with_trigger_action(t)
            .with_antidependency(c)
            .finish();

        this
    }

    fn r0(&mut self, sched: &SchedulerPoint) {
        self.i += 1;
        sched.set_port(self.c, self.i);
        if self.i >= 100_000u32 {
            sched.shutdown();
        }
    }
}

#[test]
fn count() {
    // install global collector configured based on RUST_LOG env var.
    tracing_subscriber::fmt::init();

    let mut env_builder = EnvBuilder::new();

    let count = env_builder.add_reactor_type("Count", move |mut builder| {
        let _count = Count::build(&mut builder);
        builder.finish()
    });

    let env = env_builder.build(count);
    let environment = env.build().unwrap();
    let mut sched = runtime::Scheduler::new(environment.max_level());
    sched.start(environment).unwrap();
}
