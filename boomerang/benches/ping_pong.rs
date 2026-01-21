#![allow(dead_code)]

//! Copyright (C) 2021 TU Dresden
//!
//! Micro-benchmark from the Savina benchmark suite.
//! See documentation in the C++ version.
//!
//! @author Clément Fournier

use boomerang::prelude::*;
use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput};

#[derive(Debug)]
struct Ping {
    count: usize,
    pings_left: usize,
}

impl Ping {
    fn new(count: usize) -> Self {
        Self {
            count,
            pings_left: 0,
        }
    }
}

#[reactor(state = Ping)]
fn PingReactor(
    #[input] in_start: (),
    #[input] in_pong: (),
    #[output] out_ping: (),
    #[output] out_finished: (),
) -> impl Reactor {
    let serve = builder.add_action::<(), Logical>("serve", None)?;

    reaction! {
        InStart (in_start) serve {
            // reset local state
            state.pings_left = state.count;
            // start execution
            ctx.schedule_action(&mut serve, (), None);
        }
    }

    reaction! {
        Serve (serve) -> out_ping {
            *out_ping = Some(());
            state.pings_left -= 1;
        }
    }

    reaction! {
        InPong (in_pong) serve -> out_finished {
            if state.pings_left == 0 {
                *out_finished = Some(());
            } else {
                ctx.schedule_action(&mut serve, (), None);
            }
        }
    }
}

#[derive(Default, Debug)]
struct Pong {
    count: usize,
}

#[reactor(state = Pong)]
fn PongReactor(#[input] in_ping: (), #[output] out_pong: ()) -> impl Reactor {
    reaction! {
        InPing (in_ping) -> out_pong {
            *out_pong = Some(());
            state.count += 1;
        }
    }
}

#[reactor]
fn Main(count: usize) -> impl Reactor {
    let ping = builder.add_child_reactor(PingReactor(), "ping", Ping::new(count), false)?;
    let pong = builder.add_child_reactor(PongReactor(), "pong", Pong::default(), false)?;

    builder.connect_port(ping.out_ping, pong.in_ping, None, false)?;
    builder.connect_port(pong.out_pong, ping.in_pong, None, false)?;

    reaction! {
        Startup (startup) -> ping.in_start {
            *ping_in_start = Some(());
        }
    }

    reaction! {
        Done (ping.out_finished) {
            ctx.schedule_shutdown(None);
        }
    }
}

fn bench(c: &mut Criterion) {
    #[cfg(feature = "parallel")]
    {
        eprintln!("Parallel runtime is enabled, expect poor performance.");
    }

    let mut group = c.benchmark_group("ping_pong");

    for count in [100, 10_000, 1_000_000].into_iter() {
        group.sample_size(25);
        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &count| {
            b.iter_batched(
                || {
                    let mut env_builder = EnvBuilder::new();
                    let reactor = Main(count);
                    let _reactor = reactor
                        .build("main", (), None, None, false, &mut env_builder)
                        .unwrap();
                    let BuilderRuntimeParts {
                        enclaves,
                        aliases: _,
                        ..
                    } = env_builder.into_runtime_parts().unwrap();
                    let (enclave_key, enclave) = enclaves.into_iter().next().unwrap();
                    let config = runtime::Config::default().with_fast_forward(true);
                    runtime::Scheduler::new(enclave_key, enclave, config)
                },
                |mut sched| {
                    sched.event_loop();

                    // validate the end state
                    let env = sched.into_env();
                    let ping = env
                        .find_reactor_by_name("main/ping")
                        .and_then(|reactor| reactor.get_state::<Ping>())
                        .unwrap();
                    assert_eq!(ping.count, count);
                    assert_eq!(ping.pings_left, 0);

                    let pong = env
                        .find_reactor_by_name("main/pong")
                        .and_then(|reactor| reactor.get_state::<Pong>())
                        .unwrap();
                    assert_eq!(pong.count, count);
                },
                BatchSize::SmallInput,
            );
        });
    }
}

criterion_group!(benches, bench);
criterion_main!(benches);
