//! This test checks the correctness of the cycle between two enclaves.
//!
//! Ported from LF https://github.com/lf-lang/lingua-franca/blob/master/test/Cpp/src/enclave/EnclaveCycle.lf

use std::time::Duration;

use boomerang::prelude::*;

mod ping {
    use super::*;

    #[derive(Default)]
    pub struct PingState {
        counter: i32,
        received: bool,
    }

    #[derive(Reactor)]
    #[reactor(state = PingState, reaction = "ReactionT", reaction = "ReactionIn", reaction = "ReactionShutdown")]
    pub struct Ping {
        #[reactor(timer(period = "100 ms"))]
        t: TimerActionKey,
        pub r#in: TypedPortKey<i32, Input>,
        pub out: TypedPortKey<i32, Output>,
    }

    #[derive(Reaction)]
    #[reaction(reactor = "Ping", triggers(action = "t"))]
    struct ReactionT<'a> {
        out: runtime::OutputRef<'a, i32>,
    }

    impl runtime::Trigger<PingState> for ReactionT<'_> {
        fn trigger(mut self, _ctx: &mut runtime::Context, state: &mut PingState) {
            state.counter += 1;
            *self.out = Some(state.counter);
        }
    }

    #[derive(Reaction)]
    #[reaction(reactor = "Ping")]
    struct ReactionIn<'a> {
        r#in: runtime::InputRef<'a, i32>,
    }

    impl runtime::Trigger<PingState> for ReactionIn<'_> {
        fn trigger(self, ctx: &mut runtime::Context, state: &mut PingState) {
            state.received = true;
            let value = *self.r#in;
            println!("Ping Received {value:?}");
            let expected = std::time::Duration::from_millis(50 + 100 * value.unwrap() as u64);
            assert_eq!(
                ctx.get_elapsed_logical_time(),
                expected,
                "Expecded value at {expected:?} but received it at {:?}",
                ctx.get_elapsed_logical_time()
            );
        }
    }

    #[derive(Reaction)]
    #[reaction(reactor = "Ping", triggers(shutdown))]
    struct ReactionShutdown;

    impl runtime::Trigger<PingState> for ReactionShutdown {
        fn trigger(self, _ctx: &mut runtime::Context, state: &mut PingState) {
            if !state.received {
                panic!("Nothing received.");
            }
        }
    }
}

mod pong {
    use super::*;

    #[derive(Default)]
    pub struct PongState {
        received: bool,
    }

    #[derive(Reactor)]
    #[reactor(state = PongState, reaction = "ReactionIn", reaction = "ReactionShutdown")]
    pub struct Pong {
        pub r#in: TypedPortKey<i32, Input>,
        pub out: TypedPortKey<i32, Output>,
    }

    #[derive(Reaction)]
    #[reaction(reactor = "Pong")]
    struct ReactionIn<'a> {
        r#in: runtime::InputRef<'a, i32>,
        out: runtime::OutputRef<'a, i32>,
    }

    impl runtime::Trigger<PongState> for ReactionIn<'_> {
        fn trigger(mut self, ctx: &mut runtime::Context, state: &mut PongState) {
            state.received = true;
            let value = *self.r#in;
            println!("Pong Received {value:?}");
            let expected = std::time::Duration::from_millis(100 * value.unwrap() as u64);
            assert_eq!(
                ctx.get_elapsed_logical_time(),
                expected,
                "Expecded value at {expected:?} but received it at {:?}",
                ctx.get_elapsed_logical_time()
            );
            *self.out = value;
        }
    }

    #[derive(Reaction)]
    #[reaction(reactor = "Pong", triggers(shutdown))]
    struct ReactionShutdown;

    impl runtime::Trigger<PongState> for ReactionShutdown {
        fn trigger(self, _ctx: &mut runtime::Context, state: &mut PongState) {
            if !state.received {
                panic!("Nothing received.");
            }
        }
    }
}

#[derive(Reactor)]
#[reactor(
  state = (),
  connection(from = "ping.out", to = "pong.r#in"),
  connection(from = "pong.out", to = "ping.r#in", after = "50 ms")
)]
struct Main {
    #[reactor(child(state = ping::PingState::default(), enclave))]
    ping: ping::Ping,
    #[reactor(child(state = pong::PongState::default(), enclave))]
    pong: pong::Pong,
}

#[test]
fn enclave_cycle() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default()
        .with_fast_forward(true)
        .with_timeout(Duration::from_secs(1));
    let mut env_builder = EnvBuilder::new();
    let reactor = <Main>::build("main", (), None, None, false, &mut env_builder).unwrap();

    let puml = env_builder.create_plantuml_graph().unwrap();
    std::fs::write("enclave_cycle.puml", puml).unwrap();
    //let gv = boomerang::builder::graphviz::create_full_graph(&env_builder).unwrap();
    //std::fs::write("enclave_cycle.dot", gv).unwrap();

    let mut runtime_parts = env_builder.into_runtime_parts().unwrap();
    let boomerang_builder::EnclaveParts { enclave, aliases } = runtime_parts.remove(0);
    let mut sched = boomerang_runtime::Scheduler::new(enclave, config);
    sched.event_loop();
}
