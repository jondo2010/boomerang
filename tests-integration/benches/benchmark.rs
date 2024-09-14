use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};

use boomerang::{builder::prelude::*, runtime, Reaction, Reactor};

struct HelloBench {
    my_i: u32,
}

#[derive(Clone, Reactor)]
#[reactor(
    state = HelloBench,
    connection(from = "out1", to = "in1")
)]
struct HelloBenchBuilder {
    #[reactor(timer(offset = "100 msec", period = "1 sec"))]
    tim1: TimerActionKey,

    in1: TypedPortKey<u32, Input>,
    out1: TypedPortKey<u32, Output>,

    foo: TypedReactionKey<ReactionFoo<'static>>,
    bar: TypedReactionKey<ReactionBar<'static>>,
}

#[derive(Reaction)]
#[reaction(triggers(action = "tim1"))]
struct ReactionFoo<'a> {
    out1: runtime::OutputRef<'a, u32>,
}

impl Trigger for ReactionFoo<'_> {
    type Reactor = HelloBenchBuilder;
    fn trigger(
        &mut self,
        _ctx: &mut runtime::Context,
        state: &mut <Self::Reactor as Reactor>::State,
    ) {
        state.my_i += 1;
        *self.out1 = Some(state.my_i);
    }
}

#[derive(Reaction)]
struct ReactionBar<'a> {
    in1: runtime::InputRef<'a, u32>,
}

impl Trigger for ReactionBar<'_> {
    type Reactor = HelloBenchBuilder;

    fn trigger(
        &mut self,
        ctx: &mut runtime::Context,
        _state: &mut <Self::Reactor as Reactor>::State,
    ) {
        if self.in1.unwrap() >= 10000 {
            ctx.schedule_shutdown(None);
        }
    }
}

fn bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("benchmark");

    for count in [100 /*1000, 10000 100000, 1000000*/].into_iter() {
        group.sample_size(count);
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &_count| {
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
