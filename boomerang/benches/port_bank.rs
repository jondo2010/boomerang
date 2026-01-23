#![allow(dead_code)]

use boomerang::prelude::*;
use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput};
#[cfg(not(windows))]
use pprof::criterion::{Output, PProfProfiler};

#[derive(Clone, Copy, Debug)]
enum Topology {
    Zip,
    Broadcast,
    Cartesian,
}

impl Topology {
    fn as_str(self) -> &'static str {
        match self {
            Topology::Zip => "zip",
            Topology::Broadcast => "broadcast",
            Topology::Cartesian => "cartesian",
        }
    }
}

#[derive(Default)]
struct BenchState {
    remaining: usize,
    checksum: u64,
}

#[reactor(state = BenchState)]
fn BankBench(width: usize, iterations: usize, topology: Topology) -> impl Reactor {
    let tick = builder.add_action::<(), Logical>("tick", None)?;
    let (src_width, dst_width) = match topology {
        Topology::Zip => (width, width),
        Topology::Broadcast => (1, width),
        Topology::Cartesian => (width, width),
    };

    let outputs = builder.add_output_bank::<u64>("out", src_width)?;
    let inputs = builder.add_input_bank::<u64>("in", dst_width)?;

    match topology {
        Topology::Zip => {
            builder.connect_ports(outputs.iter(), inputs.iter(), None, false)?;
        }
        Topology::Broadcast => {
            let source = outputs.get(0).expect("output bank is non-empty");
            builder.connect_broadcast(source, inputs.iter(), None, false)?;
        }
        Topology::Cartesian => {
            builder.connect_cartesian(outputs.iter(), inputs.iter(), None, false)?;
        }
    }

    builder
        .add_reaction(Some("startup"))
        .with_startup_trigger()
        .with_effect(tick)
        .with_reaction_fn(move |ctx, state, (_startup, mut tick)| {
            state.remaining = iterations;
            state.checksum = 0;
            ctx.schedule_action(&mut tick, (), None);
        })
        .finish()?;

    builder
        .add_reaction(Some("tick"))
        .with_trigger(tick)
        .with_effect(outputs)
        .with_reaction_fn(|ctx, state, (mut tick, mut outputs)| {
            for (idx, mut out) in outputs.iter_mut().enumerate() {
                *out = Some(idx as u64);
            }

            if state.remaining == 0 {
                ctx.schedule_shutdown(None);
                return;
            }

            state.remaining -= 1;
            if state.remaining == 0 {
                ctx.schedule_shutdown(None);
            } else {
                ctx.schedule_action(&mut tick, (), None);
            }
        })
        .finish()?;

    builder
        .add_reaction(Some("inputs"))
        .with_trigger(inputs)
        .with_reaction_fn(|_ctx, state, (inputs,)| {
            let sum = inputs
                .iter()
                .filter_map(|port| port.as_ref().copied())
                .sum::<u64>();
            state.checksum = state.checksum.wrapping_add(sum);
        })
        .finish()?;
}

struct Case {
    topology: Topology,
    width: usize,
    iterations: usize,
}

fn bench(c: &mut Criterion) {
    let cases = [
        Case {
            topology: Topology::Zip,
            width: 1,
            iterations: 10_000,
        },
        Case {
            topology: Topology::Zip,
            width: 4,
            iterations: 10_000,
        },
        Case {
            topology: Topology::Zip,
            width: 16,
            iterations: 5_000,
        },
        Case {
            topology: Topology::Zip,
            width: 64,
            iterations: 1_000,
        },
        Case {
            topology: Topology::Broadcast,
            width: 1,
            iterations: 10_000,
        },
        Case {
            topology: Topology::Broadcast,
            width: 4,
            iterations: 10_000,
        },
        Case {
            topology: Topology::Broadcast,
            width: 16,
            iterations: 5_000,
        },
        Case {
            topology: Topology::Broadcast,
            width: 64,
            iterations: 1_000,
        },
        Case {
            topology: Topology::Cartesian,
            width: 1,
            iterations: 5_000,
        },
    ];

    let mut group = c.benchmark_group("port_bank");
    group.sample_size(25);

    for case in cases {
        let id = BenchmarkId::new(case.topology.as_str(), case.width);
        group.throughput(Throughput::Elements(case.iterations as u64));
        group.bench_with_input(id, &case, |b, case| {
            b.iter_batched(
                || {
                    let mut env_builder = EnvBuilder::new();
                    let reactor = BankBench(case.width, case.iterations, case.topology);
                    let _reactor = reactor
                        .build(
                            "main",
                            BenchState::default(),
                            None,
                            None,
                            false,
                            &mut env_builder,
                        )
                        .unwrap();
                    let config = runtime::Config::default().with_fast_forward(true);
                    let BuilderRuntimeParts { enclaves, .. } =
                        env_builder.into_runtime_parts(&config).unwrap();
                    let (enclave_key, enclave) = enclaves.into_iter().next().unwrap();
                    runtime::Scheduler::new(enclave_key, enclave, config)
                },
                |mut sched| {
                    sched.event_loop();
                },
                BatchSize::SmallInput,
            );
        });
    }
}

fn criterion_config() -> Criterion {
    let mut criterion = Criterion::default();
    #[cfg(not(windows))]
    if std::env::var_os("BOOMERANG_PROFILE").is_some() {
        criterion = criterion.with_profiler(PProfProfiler::new(100, Output::Flamegraph(None)));
    }
    criterion
}

criterion_group! {
    name = benches;
    config = criterion_config();
    targets = bench
}
criterion_main!(benches);
