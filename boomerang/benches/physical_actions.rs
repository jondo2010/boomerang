//! A benchmark to check the throughput of scheduling and processing async physical actions.

use boomerang::prelude::*;
use criterion::{criterion_group, criterion_main, Criterion};
use std::thread::JoinHandle;

#[derive(Debug, Default)]
struct State {
    thread: Option<JoinHandle<usize>>,
    sent: usize,
    received: usize,
}

/// The Run reaction starts a new thread that sends as many actions as possible until requested to shut down. Upon shutdown, it returns the number of actions sent.
#[reactor(state = State)]
fn AsyncCallback() -> impl Reactor {
    let a = builder.add_physical_action::<u32>("a", None)?;

    reaction! {
        Run (startup) a {
            // make sure to join the old thread first
            if let Some(thread) = state.thread.take() {
                thread.join().unwrap();
            }

            let send_ctx = ctx.make_send_context();
            let a = a.to_async();

            // start new thread
            state.thread = Some(std::thread::spawn(move || {
                let mut count = 0;
                while !send_ctx.is_shutdown() {
                    send_ctx.schedule_action_async(&a, 0u32, None);
                    count += 1;
                }
                count
            }));
        }
    }

    reaction! {
        Proc (a) {
            state.received += 1;
        }
    }

    reaction! {
        Shutdown (shutdown) {
            if let Some(thread) = state.thread.take() {
                state.sent = thread.join().unwrap();
            }
        }
    }
}

fn bench(c: &mut Criterion) {
    c.bench_function("Physical Actions", |b| {
        b.iter_batched(
            || {
                let mut env_builder = EnvBuilder::new();
                let reactor = AsyncCallback();
                let _reactor = reactor
                    .build("main", State::default(), None, None, false, &mut env_builder)
                    .unwrap();
                let BuilderRuntimeParts { enclaves, .. } =
                    env_builder.into_runtime_parts().unwrap();
                let (enclave_key, enclave) = enclaves.into_iter().next().unwrap();
                let config = runtime::Config::default()
                    .with_fast_forward(false)
                    .with_timeout(runtime::Duration::seconds(1));
                runtime::Scheduler::new(enclave_key, enclave, config)
            },
            |mut sched| {
                sched.event_loop();
            },
            criterion::BatchSize::NumIterations(10),
        );
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
