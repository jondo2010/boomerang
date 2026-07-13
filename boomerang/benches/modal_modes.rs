#![allow(dead_code)]

use boomerang::prelude::*;
use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput};
#[cfg(not(windows))]
use pprof::criterion::{Output, PProfProfiler};

#[derive(Debug, Default)]
struct ModalModesState {
    remaining: usize,
    ticks: usize,
    timer_fires: usize,
}

#[reactor(state = ModalModesState)]
fn ModalModesBench(mode_count: usize, iterations: usize) -> impl Reactor {
    let tick = builder.add_logical_action::<()>("tick", None)?;

    let mut modes = Vec::with_capacity(mode_count);
    for idx in 0..mode_count {
        let kind = if idx == 0 {
            ModeKind::Initial
        } else {
            ModeKind::Normal
        };
        let mode = builder.add_mode(&format!("mode_{idx}"), kind)?;
        modes.push(mode);
    }

    for (idx, mode) in modes.into_iter().enumerate() {
        builder.in_mode(mode, |builder| {
            let timer = builder.add_timer(
                &format!("local_timer_{idx}"),
                TimerSpec {
                    offset: Some(Duration::nanoseconds(1)),
                    period: None,
                },
            )?;

            builder
                .add_reaction(Some(&format!("local_timer_{idx}")))
                .with_trigger(timer)
                .with_reaction_fn(|_ctx, state, (_timer,)| {
                    state.timer_fires += 1;
                })
                .finish()?;

            Ok(())
        })?;
    }

    builder
        .add_reaction(Some("startup"))
        .with_startup_trigger()
        .with_effect(tick)
        .with_reaction_fn(move |ctx, state, (_startup, mut tick)| {
            state.remaining = iterations;
            state.ticks = 0;
            state.timer_fires = 0;
            ctx.schedule_action(&mut tick, (), Some(Duration::nanoseconds(1)));
        })
        .finish()?;

    builder
        .add_reaction(Some("tick"))
        .with_trigger(tick)
        .with_reaction_fn(|ctx, state, (mut tick,)| {
            state.ticks += 1;
            state.remaining -= 1;

            if state.remaining == 0 {
                ctx.schedule_shutdown(None);
            } else {
                ctx.schedule_action(&mut tick, (), Some(Duration::nanoseconds(1)));
            }
        })
        .finish()?;

    builder
        .add_reaction(Some("shutdown"))
        .with_shutdown_trigger()
        .with_reaction_fn(move |_ctx, state, (_shutdown,)| {
            assert_eq!(state.ticks, iterations);
            assert_eq!(
                state.timer_fires, 1,
                "only the initial mode's local timer should fire"
            );
        })
        .finish()?;
}

struct Case {
    mode_count: usize,
    iterations: usize,
}

fn bench(c: &mut Criterion) {
    let cases = [
        Case {
            mode_count: 1,
            iterations: 10_000,
        },
        Case {
            mode_count: 32,
            iterations: 10_000,
        },
        Case {
            mode_count: 256,
            iterations: 10_000,
        },
    ];

    let mut group = c.benchmark_group("modal_modes");
    group.sample_size(10);

    for case in cases {
        let id = BenchmarkId::new("inactive_modes", case.mode_count);
        group.throughput(Throughput::Elements(case.iterations as u64));
        group.bench_with_input(id, &case, |b, case| {
            b.iter_batched(
                || {
                    let mut assembly = Assembly::new();
                    let reactor = ModalModesBench(case.mode_count, case.iterations);
                    let _reactor = reactor
                        .build(
                            "main",
                            ModalModesState::default(),
                            None,
                            None,
                            None,
                            false,
                            &mut assembly,
                        )
                        .unwrap();
                    let config = runtime::Config::default().with_fast_forward(true);
                    let BuilderRuntimeParts { enclaves, .. } =
                        assembly.into_runtime_parts(&config).unwrap();
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
