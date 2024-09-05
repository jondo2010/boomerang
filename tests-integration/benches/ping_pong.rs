#![allow(dead_code)]

//! Copyright (C) 2021 TU Dresden
//!
//! Micro-benchmark from the Savina benchmark suite.
//! See documentation in the C++ version.
//!
//! @author Clément Fournier

use boomerang::{builder::prelude::*, runtime, Reaction, Reactor};
use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput};

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

#[derive(Clone, Reactor)]
#[reactor(state = Ping)]
struct PingBuilder {
    #[reactor(port = "input")]
    in_start: TypedPortKey<()>,
    #[reactor(port = "input")]
    in_pong: TypedPortKey<()>,

    #[reactor(port = "output")]
    out_ping: TypedPortKey<()>,
    #[reactor(port = "output")]
    out_finished: TypedPortKey<()>,

    #[reactor(action(physical = "false"))]
    serve: TypedActionKey,

    reaction_in_start: TypedReactionKey<ReactionInStart<'static>>,
    reaction_in_pong: TypedReactionKey<ReactionInPong<'static>>,
    reaction_serve: TypedReactionKey<ReactionServe<'static>>,
}

#[derive(Reaction)]
#[reaction(triggers(port = "in_start"))]
struct ReactionInStart<'a> {
    serve: runtime::ActionRef<'a>,
}

impl Trigger for ReactionInStart<'_> {
    type Reactor = PingBuilder;
    fn trigger(&mut self, ctx: &mut runtime::Context, state: &mut Ping) {
        // reset local state
        state.pings_left = state.count;
        // start execution
        ctx.schedule_action(&mut self.serve, None, None);
    }
}

#[derive(Reaction)]
#[reaction(triggers(action = "serve"))]
struct ReactionServe<'a> {
    out_ping: &'a mut runtime::Port<()>,
}

impl Trigger for ReactionServe<'_> {
    type Reactor = PingBuilder;
    fn trigger(&mut self, _ctx: &mut runtime::Context, state: &mut Ping) {
        *self.out_ping.get_mut() = Some(());
        state.pings_left -= 1;
    }
}

#[derive(Reaction)]
#[reaction(triggers(port = "in_pong"))]
struct ReactionInPong<'a> {
    out_finished: &'a mut runtime::Port<()>,
    serve: runtime::ActionRef<'a>,
}

impl Trigger for ReactionInPong<'_> {
    type Reactor = PingBuilder;
    fn trigger(&mut self, ctx: &mut runtime::Context, state: &mut Ping) {
        if state.pings_left == 0 {
            *self.out_finished.get_mut() = Some(());
        } else {
            ctx.schedule_action(&mut self.serve, None, None);
        }
    }
}

#[derive(Default)]
struct Pong {
    count: usize,
}

#[derive(Clone, Reactor)]
#[reactor(state = Pong)]
struct PongBuilder {
    #[reactor(port = "input")]
    in_ping: TypedPortKey<()>,
    #[reactor(port = "output")]
    out_pong: TypedPortKey<()>,
    reaction_in_ping: TypedReactionKey<ReactionInPing<'static>>,
}

#[derive(Reaction)]
#[reaction(triggers(port = "in_ping"))]
struct ReactionInPing<'a> {
    out_pong: &'a mut runtime::Port<()>,
}

impl Trigger for ReactionInPing<'_> {
    type Reactor = PongBuilder;
    fn trigger(&mut self, _ctx: &mut runtime::Context, state: &mut Pong) {
        *self.out_pong.get_mut() = Some(());
        state.count += 1;
    }
}

#[derive(Copy, Clone)]
struct Main {
    count: usize,
}

#[derive(Clone, Reactor)]
#[reactor(
    state = Main,
    connection(from = "ping.out_ping", to = "pong.in_ping"),
    connection(from = "pong.out_pong", to = "ping.in_pong")
)]
struct MainBuilder {
    #[reactor(child= Ping::new(state.count))]
    ping: PingBuilder,

    #[reactor(child= Pong::default())]
    pong: PongBuilder,

    reaction_startup: TypedReactionKey<ReactionStartup<'static>>,
    reaction_done: TypedReactionKey<ReactionDone<'static>>,
}

#[derive(Reaction)]
#[reaction(triggers(startup))]
struct ReactionStartup<'a> {
    #[reaction(path = "ping.in_start")]
    in_start: &'a mut runtime::Port<()>,
}

impl Trigger for ReactionStartup<'_> {
    type Reactor = MainBuilder;
    fn trigger(&mut self, _ctx: &mut runtime::Context, _state: &mut Main) {
        *self.in_start.get_mut() = Some(());
    }
}

#[derive(Reaction)]
struct ReactionDone<'a> {
    #[reaction(path = "ping.out_finished")]
    _out: &'a runtime::Port<()>,
}

impl Trigger for ReactionDone<'_> {
    type Reactor = MainBuilder;
    fn trigger(&mut self, ctx: &mut runtime::Context, _state: &mut Main) {
        ctx.schedule_shutdown(None);
    }
}

fn bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("ping_pong");

    for count in [100, 10000, 1000000].into_iter() {
        group.sample_size(25);
        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &count| {
            b.iter_batched(
                || {
                    let mut env_builder = EnvBuilder::new();
                    let _reactor =
                        MainBuilder::build("main", Main { count }, None, &mut env_builder).unwrap();
                    let (env, triggers, _) = env_builder.into_runtime_parts().unwrap();
                    (env, triggers)
                },
                |(mut env, triggers)| {
                    let mut sched = runtime::Scheduler::new(&mut env, triggers, true, false);
                    sched.event_loop();

                    let ping = env
                        .get_reactor_by_name("ping")
                        .and_then(|reactor| reactor.get_state::<Ping>())
                        .unwrap();
                    assert_eq!(ping.count, count);
                    assert_eq!(ping.pings_left, 0);

                    let pong = env
                        .get_reactor_by_name("pong")
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