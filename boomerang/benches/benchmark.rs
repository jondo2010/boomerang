use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};

use boomerang::prelude::*;

struct HelloBench {
    my_i: u32,
}

#[reactor(state = HelloBench)]
fn HelloBenchBuilder(#[input] in1: u32, #[output] out1: u32) -> impl Reactor {
    timer! { tim1(100 msec, 1 sec) };

    builder.connect_port(out1, in1, None, false)?;

    reaction! {
        ReactionFoo (tim1) -> out1 {
            state.my_i += 1;
            *out1 = Some(state.my_i);
        }
    }

    reaction! {
        ReactionBar (in1) {
            if in1.unwrap() >= 10000 {
                ctx.schedule_shutdown(None);
            }
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
                    let mut assembly = Assembly::new();
                    let reactor = HelloBenchBuilder();
                    let _reactor = reactor
                        .build(
                            "benchmark",
                            HelloBench { my_i: 0 },
                            None,
                            None,
                            None,
                            false,
                            &mut assembly,
                        )
                        .unwrap();
                    let config = runtime::Config::default().with_fast_forward(true);
                    let RuntimeAssembly { enclaves, .. } =
                        assembly.into_runtime_assembly(&config).unwrap();
                    let (enclave_key, enclave) = enclaves.into_iter().next().unwrap();
                    runtime::Scheduler::new(enclave_key, enclave, config)
                },
                |mut sched| {
                    sched.try_event_loop().unwrap();
                },
                BatchSize::SmallInput,
            );
        });
    }
}

criterion_group!(benches, bench);
criterion_main!(benches);
