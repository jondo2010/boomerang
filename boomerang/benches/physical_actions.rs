//! A benchmark to check the throughput of scheduling and processing async physical actions.

use boomerang::prelude::*;
use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput};
#[cfg(not(windows))]
use pprof::criterion::{Output, PProfProfiler};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

#[derive(Debug, Default)]
struct State {
    received: Arc<AtomicUsize>,
}

#[reactor(state = State)]
fn AsyncCallback() -> impl Reactor {
    let a = builder.add_physical_action::<u32>("a", None)?;

    reaction! {
        Proc (a) {
            state.received.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("physical_actions");
    for count in [1_000, 10_000, 100_000] {
        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &count| {
            b.iter_batched(
                || {
                    let received = Arc::new(AtomicUsize::new(0));
                    let mut env_builder = EnvBuilder::new();
                    let reactor = AsyncCallback();
                    let _reactor = reactor
                        .build(
                            "main",
                            State {
                                received: received.clone(),
                            },
                            None,
                            None,
                            false,
                            &mut env_builder,
                        )
                        .unwrap();
                    let action_key = env_builder
                        .find_physical_action_by_fqn("main/a")
                        .unwrap();
                    let config = runtime::Config::default()
                        .with_fast_forward(false)
                        .with_keep_alive(true)
                        .with_queue_size(65_536);
                    let BuilderRuntimeParts { enclaves, aliases, .. } =
                        env_builder.into_runtime_parts(&config).unwrap();
                    let (enclave_key, enclave) = enclaves.into_iter().next().unwrap();
                    let (action_enclave_key, action_key) = aliases.action_aliases[action_key];
                    assert_eq!(
                        action_enclave_key, enclave_key,
                        "physical action enclave mismatch"
                    );
                    let send_ctx = enclave.create_send_context(enclave_key);
                    let action_ref = enclave.create_async_action_ref::<u32>(action_key);
                    let mut scheduler = runtime::Scheduler::new(enclave_key, enclave, config);
                    let scheduler_thread = std::thread::spawn(move || {
                        scheduler.event_loop();
                    });

                    (send_ctx, action_ref, scheduler_thread, received)
                },
                |(mut send_ctx, action_ref, scheduler_thread, received)| {
                    for i in 0..count {
                        let delay = runtime::Duration::nanoseconds(i as i64);
                        let scheduled =
                            send_ctx.schedule_action_async(&action_ref, 0u32, Some(delay));
                        assert!(scheduled, "failed to schedule physical action");
                    }

                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                    while received.load(Ordering::Relaxed) < count {
                        if std::time::Instant::now() >= deadline {
                            panic!(
                                "timed out waiting for physical actions (received {}, expected {})",
                                received.load(Ordering::Relaxed),
                                count
                            );
                        }
                        std::thread::yield_now();
                    }

                    send_ctx.schedule_shutdown(None);
                    scheduler_thread.join().unwrap();
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
        criterion = criterion
            .with_profiler(PProfProfiler::new(100, Output::Flamegraph(None)))
            .profile_time(Some(std::time::Duration::from_secs(5)));
    }
    criterion
}

criterion_group! {
    name = benches;
    config = criterion_config();
    targets = bench
}
criterion_main!(benches);
