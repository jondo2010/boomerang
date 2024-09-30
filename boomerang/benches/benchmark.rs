use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};

use boomerang::prelude::*;

struct HelloBench {
    my_i: u32,
}

#[derive(Reactor)]
#[reactor(
    state = HelloBench,
    connection(from = "out1", to = "in1"),
    reaction = "ReactionFoo",
    reaction = "ReactionBar"
)]
struct HelloBenchBuilder {
    #[reactor(timer(offset = "100 msec", period = "1 sec"))]
    tim1: TimerActionKey,

    in1: TypedPortKey<u32, Input>,
    out1: TypedPortKey<u32, Output>,
}

#[derive(Reaction)]
#[reaction(reactor = "HelloBenchBuilder", triggers(action = "tim1"))]
struct ReactionFoo<'a> {
    out1: runtime::OutputRef<'a, u32>,
}

impl Trigger<HelloBenchBuilder> for ReactionFoo<'_> {
    fn trigger(mut self, _ctx: &mut runtime::Context, state: &mut HelloBench) {
        state.my_i += 1;
        *self.out1 = Some(state.my_i);
    }
}

#[derive(Reaction)]
#[reaction(reactor = "HelloBenchBuilder")]
struct ReactionBar<'a> {
    in1: runtime::InputRef<'a, u32>,
}

impl Trigger<HelloBenchBuilder> for ReactionBar<'_> {
    fn trigger(self, ctx: &mut runtime::Context, _state: &mut HelloBench) {
        if self.in1.unwrap() >= 10000 {
            ctx.schedule_shutdown(None);
        }
    }
}

fn bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("benchmark");

    for count in [100, 1000 /*10000 100000, 1000000*/].into_iter() {
        group.sample_size(count);
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &_count| {
            b.iter_batched(
                || {
                    let mut env_builder = EnvBuilder::new();
                    let _reactor = HelloBenchBuilder::build(
                        "benchmark",
                        HelloBench { my_i: 0 },
                        None,
                        None,
                        &mut env_builder,
                    )
                    .unwrap();
                    let (env, triggers, _) = env_builder.into_runtime_parts().unwrap();
                    (env, triggers)
                },
                |(env, triggers)| {
                    let config = runtime::Config::default().with_fast_forward(true);
                    let mut sched = runtime::Scheduler::new(env, triggers, config);
                    sched.event_loop();
                },
                BatchSize::SmallInput,
            );
        });
    }
}

criterion_group!(benches, bench);
criterion_main!(benches);
