use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};

use boomerang::{
    builder::{BuilderReactionKey, EnvBuilder, Reactor, TypedActionKey, TypedPortKey},
    runtime, Reactor,
};

#[derive(Reactor)]
#[reactor(state = "HelloBench", connection(from = "out1", to = "in1"))]
struct HelloBenchBuilder {
    #[reactor(timer(offset = "100 msec", period = "1 sec"))]
    tim1: TypedActionKey,

    #[reactor(input())]
    in1: TypedPortKey<u32>,

    #[reactor(output())]
    out1: TypedPortKey<u32>,

    #[reactor(reaction(function = "HelloBench::foo"))]
    foo: BuilderReactionKey,

    #[reactor(reaction(function = "HelloBench::bar"))]
    bar: BuilderReactionKey,
}

struct HelloBench {
    my_i: u32,
}

impl HelloBench {
    #[boomerang::reaction(reactor = "HelloBenchBuilder", triggers(action = "tim1"))]
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

fn bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("benchmark");

    for count in [100 /*1000, 10000 100000, 1000000*/].into_iter() {
        group.sample_size(count);
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &count| {
            b.iter_batched(
                || {
                    let mut env_builder = EnvBuilder::new();
                    let _reactor = HelloBenchBuilder::build(
                        "benchmark",
                        HelloBench { my_i: 0 },
                        None,
                        &mut env_builder,
                    )
                    .unwrap();
                    let (env, triggers, _) = env_builder.into_runtime_parts().unwrap();
                    (env, triggers)
                },
                |(mut env, triggers)| {
                    let mut sched = runtime::Scheduler::new(&mut env, triggers, true, false);
                    sched.event_loop();
                },
                BatchSize::SmallInput,
            );
        });
    }
}

criterion_group!(benches, bench);
criterion_main!(benches);
