use criterion::{criterion_group, criterion_main, Criterion};

use boomerang::{
    builder::{BuilderActionKey, BuilderPortKey, EnvBuilder, Reactor},
    runtime, Reactor,
};

#[derive(Reactor)]
#[reactor(connection(from = "out1", to = "in1"))]
struct HelloBenchBuilder {
    #[reactor(timer(offset = "100 msec", period = "1 sec"))]
    tim1: BuilderActionKey,
    #[reactor(input())]
    in1: BuilderPortKey<u32>,
    #[reactor(output())]
    out1: BuilderPortKey<u32>,
    #[reactor(reaction(function = "HelloBench::foo"))]
    foo: runtime::ReactionKey,
    #[reactor(reaction(function = "HelloBench::bar"))]
    bar: runtime::ReactionKey,
}

struct HelloBench {
    my_i: u32,
}

impl HelloBench {
    #[boomerang::reaction(reactor = "HelloBenchBuilder", triggers(timer = "tim1"))]
    fn foo(
        &mut self,
        _ctx: &mut runtime::Context,
        #[reactor::port(effects)] out1: &mut runtime::Port<u32>,
    ) {
        self.my_i += 1;
        **out1 = Some(self.my_i);
    }

    #[boomerang::reaction(reactor = "HelloBenchBuilder")]
    fn bar(
        &mut self,
        ctx: &mut runtime::Context,
        #[reactor::port(triggers)] in1: &runtime::Port<u32>,
    ) {
        if in1.get().unwrap() >= 10000 {
            ctx.schedule_shutdown(None);
        }
    }
}

#[inline]
fn benchmark() {
    let mut env_builder = EnvBuilder::new();
    let _ = HelloBenchBuilder::build("benchmark", HelloBench { my_i: 0 }, None, &mut env_builder)
        .expect("Error building top-level reactor!");
    let (env, dep_info) = env_builder.try_into().unwrap();
    let sched = runtime::Scheduler::new(env, dep_info, true);
    sched.event_loop();
}

pub fn criterion_benchmark(c: &mut Criterion) {
    c.bench_function("benchmark", |b| b.iter(|| benchmark()));
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
