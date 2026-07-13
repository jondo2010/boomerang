#![allow(dead_code)]

use boomerang::prelude::*;
use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput};
#[cfg(not(windows))]
use pprof::criterion::{Output, PProfProfiler};

#[derive(Clone, Copy, Debug)]
enum TransitionCase {
    Reset,
    History,
}

impl TransitionCase {
    fn as_str(self) -> &'static str {
        match self {
            TransitionCase::Reset => "reset",
            TransitionCase::History => "history",
        }
    }
}

#[derive(Debug, Default)]
struct TransitionChurnState {
    ticks: usize,
    mode_a_ticks: usize,
    mode_b_ticks: usize,
}

#[reactor(state = TransitionChurnState)]
fn TransitionChurnBench(iterations: usize, transition: TransitionCase) -> impl Reactor {
    let tick = ctx.add_logical_action::<()>("tick", None)?;

    let mode_a = ctx.add_mode("mode_a", ModeKind::Initial)?;
    let mode_b = ctx.add_mode("mode_b", ModeKind::Normal)?;

    let a_to_b = match transition {
        TransitionCase::Reset => ctx.reset_mode_effect(mode_b)?,
        TransitionCase::History => ctx.history_mode_effect(mode_b)?,
    };
    let b_to_a = match transition {
        TransitionCase::Reset => ctx.reset_mode_effect(mode_a)?,
        TransitionCase::History => ctx.history_mode_effect(mode_a)?,
    };

    ctx.in_mode(mode_a, |ctx| {
        ctx.add_reaction(Some("mode_a_tick"))
            .with_trigger(tick)
            .with_effect(a_to_b)
            .with_reaction_fn(move |ctx, state, (mut tick, mode_b)| {
                state.ticks += 1;
                state.mode_a_ticks += 1;

                if state.ticks == iterations {
                    ctx.schedule_shutdown(None);
                } else {
                    ctx.schedule_action(&mut tick, (), Some(Duration::nanoseconds(1)));
                    mode_b.set(ctx);
                }
            })
            .finish()?;
        Ok(())
    })?;

    ctx.in_mode(mode_b, |ctx| {
        ctx.add_reaction(Some("mode_b_tick"))
            .with_trigger(tick)
            .with_effect(b_to_a)
            .with_reaction_fn(move |ctx, state, (mut tick, mode_a)| {
                state.ticks += 1;
                state.mode_b_ticks += 1;

                if state.ticks == iterations {
                    ctx.schedule_shutdown(None);
                } else {
                    ctx.schedule_action(&mut tick, (), Some(Duration::nanoseconds(1)));
                    mode_a.set(ctx);
                }
            })
            .finish()?;
        Ok(())
    })?;

    ctx.add_reaction(Some("startup"))
        .with_startup_trigger()
        .with_effect(tick)
        .with_reaction_fn(|ctx, state, (_startup, mut tick)| {
            state.ticks = 0;
            state.mode_a_ticks = 0;
            state.mode_b_ticks = 0;
            ctx.schedule_action(&mut tick, (), Some(Duration::nanoseconds(1)));
        })
        .finish()?;

    ctx.add_reaction(Some("shutdown"))
        .with_shutdown_trigger()
        .with_reaction_fn(move |_ctx, state, (_shutdown,)| {
            assert_eq!(state.ticks, iterations);
            assert!(state.mode_a_ticks > 0);
            if iterations > 1 {
                assert!(state.mode_b_ticks > 0);
            }
        })
        .finish()?;
}

#[derive(Debug, Default)]
struct FanoutParentState {
    remaining: usize,
    sent: usize,
}

#[derive(Debug, Default)]
struct FanoutChildState {
    hits: usize,
    checksum: u64,
}

#[reactor(state = FanoutChildState)]
fn FanoutChild(mode_count: usize, iterations: usize, #[input] input: u64) -> impl Reactor {
    let mut modes = Vec::with_capacity(mode_count);
    for idx in 0..mode_count {
        let kind = if idx == 0 {
            ModeKind::Initial
        } else {
            ModeKind::Normal
        };
        modes.push(ctx.add_mode(&format!("mode_{idx}"), kind)?);
    }

    for (idx, mode) in modes.into_iter().enumerate() {
        ctx.in_mode(mode, |ctx| {
            ctx.add_reaction(Some(&format!("input_mode_{idx}")))
                .with_trigger(input)
                .with_reaction_fn(|_ctx, state, (input,)| {
                    if let Some(value) = input.as_ref().copied() {
                        state.hits += 1;
                        state.checksum = state.checksum.wrapping_add(value);
                    }
                })
                .finish()?;
            Ok(())
        })?;
    }

    ctx.add_reaction(Some("shutdown"))
        .with_shutdown_trigger()
        .with_reaction_fn(move |_ctx, state, (_shutdown,)| {
            assert_eq!(
                state.hits, iterations,
                "only the initial mode's input reaction should run"
            );
            assert_ne!(state.checksum, 0);
        })
        .finish()?;
}

#[reactor(state = FanoutParentState)]
fn InactivePortFanoutBench(mode_count: usize, iterations: usize) -> impl Reactor {
    let tick = ctx.add_logical_action::<()>("tick", None)?;
    let child = ctx.add_child_reactor(
        FanoutChild(mode_count, iterations),
        "child",
        FanoutChildState::default(),
        false,
    )?;

    ctx.add_reaction(Some("startup"))
        .with_startup_trigger()
        .with_effect(tick)
        .with_reaction_fn(move |ctx, state, (_startup, mut tick)| {
            state.remaining = iterations;
            state.sent = 0;
            ctx.schedule_action(&mut tick, (), Some(Duration::nanoseconds(1)));
        })
        .finish()?;

    ctx.add_reaction(Some("tick"))
        .with_trigger(tick)
        .with_effect(child.input)
        .with_reaction_fn(|ctx, state, (mut tick, mut input)| {
            state.sent += 1;
            *input = Some(state.sent as u64);

            state.remaining -= 1;
            if state.remaining == 0 {
                ctx.schedule_shutdown(None);
            } else {
                ctx.schedule_action(&mut tick, (), Some(Duration::nanoseconds(1)));
            }
        })
        .finish()?;

    ctx.add_reaction(Some("shutdown"))
        .with_shutdown_trigger()
        .with_reaction_fn(move |_ctx, state, (_shutdown,)| {
            assert_eq!(state.sent, iterations);
        })
        .finish()?;
}

#[reactor]
fn EmptyScopedChild() -> impl Reactor {}

#[derive(Debug, Default)]
struct ResetSubtreeState {
    remaining: usize,
    resets: usize,
}

#[reactor(state = ResetSubtreeState)]
fn ResetSubtreeBench(timer_count: usize, child_count: usize, iterations: usize) -> impl Reactor {
    let tick = ctx.add_logical_action::<()>("tick", None)?;

    let idle = ctx.add_mode("idle", ModeKind::Initial)?;
    let active = ctx.add_mode("active", ModeKind::Normal)?;
    let enter_active = ctx.reset_mode_effect(active)?;
    let reset_active = ctx.reset_mode_effect(active)?;

    ctx.in_mode(idle, |ctx| {
        ctx.add_reaction(Some("enter_active"))
            .with_trigger(tick)
            .with_effect(enter_active)
            .with_reaction_fn(|ctx, state, (mut tick, active)| {
                if state.remaining == 0 {
                    ctx.schedule_shutdown(None);
                } else {
                    ctx.schedule_action(&mut tick, (), Some(Duration::nanoseconds(1)));
                    active.set(ctx);
                }
            })
            .finish()?;
        Ok(())
    })?;

    ctx.in_mode(active, |ctx| {
        for idx in 0..timer_count {
            let timer = ctx.add_timer(
                &format!("local_timer_{idx}"),
                TimerSpec {
                    offset: Some(Duration::seconds(60)),
                    period: None,
                },
            )?;
            ctx.add_reaction(Some(&format!("local_timer_{idx}")))
                .with_trigger(timer)
                .with_reaction_fn(|_ctx, _state, (_timer,)| {})
                .finish()?;
        }

        for idx in 0..child_count {
            let _child =
                ctx.add_child_reactor(EmptyScopedChild(), &format!("child_{idx}"), (), false)?;
        }

        ctx.add_reaction(Some("reset_active"))
            .with_trigger(tick)
            .with_effect(reset_active)
            .with_reaction_fn(move |ctx, state, (mut tick, active)| {
                state.resets += 1;

                state.remaining -= 1;
                if state.remaining == 0 {
                    ctx.schedule_shutdown(None);
                } else {
                    ctx.schedule_action(&mut tick, (), Some(Duration::nanoseconds(1)));
                    active.set(ctx);
                }
            })
            .finish()?;
        Ok(())
    })?;

    ctx.add_reaction(Some("startup"))
        .with_startup_trigger()
        .with_effect(tick)
        .with_reaction_fn(move |ctx, state, (_startup, mut tick)| {
            state.remaining = iterations.saturating_sub(1);
            state.resets = 0;
            ctx.schedule_action(&mut tick, (), Some(Duration::nanoseconds(1)));
        })
        .finish()?;

    ctx.add_reaction(Some("shutdown"))
        .with_shutdown_trigger()
        .with_reaction_fn(move |_ctx, state, (_shutdown,)| {
            assert_eq!(state.resets, iterations.saturating_sub(1));
        })
        .finish()?;
}

fn build_scheduler<R, State>(reactor: R, state: State) -> runtime::Scheduler
where
    R: Reactor<State>,
    State: runtime::ReactorData,
{
    let mut assembly = Assembly::new();
    let _reactor = reactor
        .build("main", state, None, None, None, false, &mut assembly)
        .unwrap();
    let config = runtime::Config::default().with_fast_forward(true);
    let RuntimeAssembly { enclaves, .. } = assembly.into_runtime_assembly(&config).unwrap();
    let (enclave_key, enclave) = enclaves.into_iter().next().unwrap();
    runtime::Scheduler::new(enclave_key, enclave, config)
}

fn bench_transition_churn(c: &mut Criterion) {
    struct Case {
        transition: TransitionCase,
        iterations: usize,
    }

    let cases = [
        Case {
            transition: TransitionCase::Reset,
            iterations: 10_000,
        },
        Case {
            transition: TransitionCase::History,
            iterations: 10_000,
        },
        Case {
            transition: TransitionCase::Reset,
            iterations: 100_000,
        },
        Case {
            transition: TransitionCase::History,
            iterations: 100_000,
        },
    ];

    let mut group = c.benchmark_group("transition_churn");
    group.sample_size(10);

    for case in cases {
        let id = BenchmarkId::new(case.transition.as_str(), case.iterations);
        group.throughput(Throughput::Elements(case.iterations as u64));
        group.bench_with_input(id, &case, |b, case| {
            b.iter_batched(
                || {
                    build_scheduler(
                        TransitionChurnBench(case.iterations, case.transition),
                        TransitionChurnState::default(),
                    )
                },
                |mut sched| sched.try_event_loop().unwrap(),
                BatchSize::SmallInput,
            );
        });
    }
}

fn bench_inactive_port_fanout(c: &mut Criterion) {
    struct Case {
        mode_count: usize,
        iterations: usize,
    }

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
            iterations: 5_000,
        },
        Case {
            mode_count: 1024,
            iterations: 1_000,
        },
    ];

    let mut group = c.benchmark_group("inactive_port_fanout");
    group.sample_size(10);

    for case in cases {
        let id = BenchmarkId::new("modes", case.mode_count);
        group.throughput(Throughput::Elements(case.iterations as u64));
        group.bench_with_input(id, &case, |b, case| {
            b.iter_batched(
                || {
                    build_scheduler(
                        InactivePortFanoutBench(case.mode_count, case.iterations),
                        FanoutParentState::default(),
                    )
                },
                |mut sched| sched.try_event_loop().unwrap(),
                BatchSize::SmallInput,
            );
        });
    }
}

fn bench_reset_subtree(c: &mut Criterion) {
    struct Case {
        name: &'static str,
        timer_count: usize,
        child_count: usize,
        iterations: usize,
    }

    let cases = [
        Case {
            name: "small",
            timer_count: 1,
            child_count: 0,
            iterations: 10_000,
        },
        Case {
            name: "medium",
            timer_count: 32,
            child_count: 32,
            iterations: 5_000,
        },
        Case {
            name: "large",
            timer_count: 256,
            child_count: 256,
            iterations: 1_000,
        },
    ];

    let mut group = c.benchmark_group("reset_subtree");
    group.sample_size(10);

    for case in cases {
        let id = BenchmarkId::new(case.name, case.timer_count + case.child_count);
        group.throughput(Throughput::Elements(case.iterations as u64));
        group.bench_with_input(id, &case, |b, case| {
            b.iter_batched(
                || {
                    build_scheduler(
                        ResetSubtreeBench(case.timer_count, case.child_count, case.iterations),
                        ResetSubtreeState::default(),
                    )
                },
                |mut sched| sched.try_event_loop().unwrap(),
                BatchSize::SmallInput,
            );
        });
    }
}

fn bench(c: &mut Criterion) {
    bench_transition_churn(c);
    bench_inactive_port_fanout(c);
    bench_reset_subtree(c);
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
