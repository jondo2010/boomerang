//! A benchmark to check the throughput of scheduling and processing async physical actions.

use boomerang::prelude::*;
use boomerang_runtime::ReactorData;
use criterion::{criterion_group, criterion_main, Criterion};
use std::{thread::JoinHandle, time::Duration};

#[derive(Debug, Default)]
struct State {
    thread: Option<JoinHandle<usize>>,
    sent: usize,
    received: usize,
}

#[derive(Reactor)]
#[reactor(
    state = "State",
    reaction = "ReactionRun<T>",
    reaction = "ReactionProc<T>",
    reaction = "ReactionShutdown"
)]
struct AsyncCallback<T: ReactorData + Default> {
    a: TypedActionKey<T, Physical>,
}

/// The Run reaction starts a new thread that sends as many actions as possible until requested to shut down. Upon shutdown, it returns the number of actions sent.
#[derive(Reaction)]
#[reaction(reactor = "AsyncCallback<T>", triggers(startup))]
struct ReactionRun<T: ReactorData + Default> {
    a: runtime::AsyncActionRef<T>,
}

impl<T: ReactorData + Default> runtime::Trigger<State> for ReactionRun<T> {
    fn trigger(self, ctx: &mut runtime::Context, state: &mut State) {
        // make sure to join the old thread first
        if let Some(thread) = state.thread.take() {
            thread.join().unwrap();
        }

        let send_ctx = ctx.make_send_context();
        let a = self.a; //.clone();

        // start new thread
        state.thread = Some(std::thread::spawn(move || {
            let mut count = 0;
            while !send_ctx.is_shutdown() {
                a.schedule(&send_ctx, T::default(), None);
                count += 1;
            }
            count
        }));
    }
}

/// The Proc reaction processes the actions by incrementing the count of actions sent.
#[derive(Reaction)]
#[reaction(reactor = "AsyncCallback<T>", triggers(action = "a"))]
struct ReactionProc<T: ReactorData + Default> {
    a: runtime::AsyncActionRef<T>,
}

impl<T: ReactorData + Default> runtime::Trigger<State> for ReactionProc<T> {
    fn trigger(self, _ctx: &mut runtime::Context, state: &mut State) {
        state.received += 1;
    }
}

/// The Shutdown reaction joins the thread and returns the number of actions sent.
#[derive(Reaction)]
#[reaction(
    reactor = "AsyncCallback<T>",
    bound = "T: ReactorData + Default",
    triggers(shutdown)
)]
struct ReactionShutdown;

impl runtime::Trigger<State> for ReactionShutdown {
    fn trigger(self, ctx: &mut runtime::Context, state: &mut State) {
        if let Some(thread) = state.thread.take() {
            state.sent = thread.join().unwrap();
        }
    }
}

fn bench(c: &mut Criterion) {
    c.bench_function("Physical Actions", |b| {
        b.iter_batched(
            || {
                let mut env_builder = EnvBuilder::new();
                let _reactor = AsyncCallback::<u32>::build(
                    "main",
                    State::default(),
                    None,
                    None,
                    &mut env_builder,
                )
                .unwrap();
                let (env, triggers, _) = env_builder.into_runtime_parts().unwrap();
                (env, triggers)
            },
            |(env, triggers)| {
                let config = runtime::Config::default()
                    .with_fast_forward(false)
                    .with_timeout(Duration::seconds(1));
                let mut sched = runtime::Scheduler::new(env, triggers, config);
                sched.event_loop();
            },
            criterion::BatchSize::NumIterations(10),
        );
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
