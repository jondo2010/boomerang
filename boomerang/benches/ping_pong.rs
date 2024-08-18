#![allow(dead_code)]

//! Copyright (C) 2021 TU Dresden
//!
//! Micro-benchmark from the Savina benchmark suite.
//! See documentation in the C++ version.
//!
//! @author Clément Fournier

use boomerang::{
    builder::{BuilderReactionKey, EnvBuilder, Reactor, TypedActionKey, TypedPortKey},
    runtime, Reactor,
};
use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput};

#[derive(Reactor)]
#[reactor(state = "Ping")]
struct PingBuilder {
    #[reactor(input())]
    in_start: TypedPortKey<()>,
    #[reactor(input())]
    in_pong: TypedPortKey<()>,

    #[reactor(output())]
    out_ping: TypedPortKey<()>,
    #[reactor(output())]
    out_finished: TypedPortKey<()>,

    #[reactor(action(physical = "false"))]
    serve: TypedActionKey<()>,

    #[reactor(reaction(function = "Ping::in_start"))]
    reaction_in_start: BuilderReactionKey,
    #[reactor(reaction(function = "Ping::in_pong"))]
    reaction_in_pong: BuilderReactionKey,
    #[reactor(reaction(function = "Ping::serve"))]
    reaction_serve: BuilderReactionKey,
}

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

    #[boomerang::reaction(reactor = "PingBuilder", triggers(port = "in_start"))]
    fn in_start(
        &mut self,
        ctx: &mut runtime::Context,
        #[reactor::action(effects)] mut serve: runtime::ActionRef<()>,
    ) {
        // reset local state
        self.pings_left = self.count;
        // start execution
        ctx.schedule_action(&mut serve, None, None);
    }

    #[boomerang::reaction(reactor = "PingBuilder", triggers(action = "serve"))]
    fn serve(
        &mut self,
        _ctx: &mut runtime::Context,
        #[reactor::port(effects)] out_ping: &mut runtime::Port<()>,
    ) {
        *out_ping.get_mut() = Some(());
        self.pings_left -= 1;
    }

    #[boomerang::reaction(reactor = "PingBuilder", triggers(port = "in_pong"))]
    fn in_pong(
        &mut self,
        ctx: &mut runtime::Context,
        #[reactor::port(effects)] out_finished: &mut runtime::Port<()>,
        #[reactor::action(effects)] mut serve: runtime::ActionRef<()>,
    ) {
        if self.pings_left == 0 {
            *out_finished.get_mut() = Some(());
        } else {
            ctx.schedule_action(&mut serve, None, None);
        }
    }
}

#[derive(Reactor)]
#[reactor(state = "Pong")]
struct PongBuilder {
    #[reactor(input())]
    in_ping: TypedPortKey<()>,
    #[reactor(output())]
    out_pong: TypedPortKey<()>,
    #[reactor(reaction(function = "Pong::in_ping"))]
    reaction_in_ping: BuilderReactionKey,
}

#[derive(Default)]
struct Pong {
    count: usize,
}

impl Pong {
    #[boomerang::reaction(reactor = "PongBuilder", triggers(port = "in_ping"))]
    fn in_ping(
        &mut self,
        _ctx: &mut runtime::Context,
        #[reactor::port(effects)] out_pong: &mut runtime::Port<()>,
    ) {
        *out_pong.get_mut() = Some(());
        self.count += 1;
    }
}

#[derive(Reactor)]
#[reactor(
    state = "Main",
    connection(from = "ping.out_ping", to = "pong.in_ping"),
    connection(from = "pong.out_pong", to = "ping.in_pong")
)]
struct MainBuilder {
    #[reactor(child(state = "Ping::new(state.count)"))]
    ping: PingBuilder,

    #[reactor(child(state = "Pong::default()"))]
    pong: PongBuilder,

    #[reactor(reaction(function = "Main::startup"))]
    reaction_startup: BuilderReactionKey,

    #[reactor(reaction(function = "Main::done"))]
    reaction_done: BuilderReactionKey,
}

struct Main {
    count: usize,
}

impl Main {
    #[boomerang::reaction(reactor = "MainBuilder", triggers(startup))]
    fn startup(
        &mut self,
        _ctx: &runtime::Context,
        #[reactor::port(effects, path = "ping.in_start")] in_start: &mut runtime::Port<()>,
    ) {
        // println!("PingPongBenchmark");
        *in_start.get_mut() = Some(());
    }

    #[boomerang::reaction(reactor = "MainBuilder")]
    fn done(
        &mut self,
        ctx: &mut runtime::Context,
        #[reactor::port(triggers, path = "ping.out_finished")] _out: &runtime::Port<()>,
    ) {
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
