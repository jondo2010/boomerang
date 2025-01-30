//! This test checks the correctness of the cycle between two enclaves.
//!
//! Ported from LF https://github.com/lf-lang/lingua-franca/blob/master/test/Cpp/src/enclave/EnclaveCycle.lf

use boomerang::prelude::*;

mod ping {
    use super::*;

    #[derive(Default)]
    pub struct PingState {
        pub counter: i32,
        pub received: bool,
    }

    #[derive(Reactor)]
    #[reactor(state = PingState, reaction = "ReactionT", reaction = "ReactionIn", reaction = "ReactionShutdown")]
    pub struct Ping {
        #[reactor(timer(period = "100 ms"))]
        t: TimerActionKey,
        pub input: TypedPortKey<i32, Input>,
        pub output: TypedPortKey<i32, Output>,
    }

    #[derive(Reaction)]
    #[reaction(reactor = "Ping", triggers(action = "t"))]
    struct ReactionT<'a> {
        output: runtime::OutputRef<'a, i32>,
    }

    impl runtime::Trigger<PingState> for ReactionT<'_> {
        fn trigger(mut self, ctx: &mut runtime::Context, state: &mut PingState) {
            let elapsed = ctx.get_elapsed_logical_time();
            println!("Ping Sent {} at {elapsed}", state.counter);
            *self.output = Some(state.counter);
            state.counter += 1;
        }
    }

    #[derive(Reaction)]
    #[reaction(reactor = "Ping")]
    struct ReactionIn<'a> {
        input: runtime::InputRef<'a, i32>,
    }

    impl runtime::Trigger<PingState> for ReactionIn<'_> {
        fn trigger(self, ctx: &mut runtime::Context, state: &mut PingState) {
            state.received = true;
            let value = *self.input;
            let elapsed = ctx.get_elapsed_logical_time();
            println!("Ping Received {value:?} at {elapsed}");
            let expected = Duration::milliseconds(50 + 100 * value.unwrap() as i64);
            assert_eq!(
                elapsed, expected,
                "Ping expected value at {expected} but received it at {elapsed}",
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
        pub input: TypedPortKey<i32, Input>,
        pub output: TypedPortKey<i32, Output>,
    }

    #[derive(Reaction)]
    #[reaction(reactor = "Pong")]
    struct ReactionIn<'a> {
        input: runtime::InputRef<'a, i32>,
        output: runtime::OutputRef<'a, i32>,
    }

    impl runtime::Trigger<PongState> for ReactionIn<'_> {
        fn trigger(mut self, ctx: &mut runtime::Context, state: &mut PongState) {
            state.received = true;
            let value = *self.input;
            let elapsed = ctx.get_elapsed_logical_time();
            println!("Pong Received {value:?} at {elapsed}");
            let expected = Duration::milliseconds(100 * value.unwrap() as i64);
            assert_eq!(
                elapsed, expected,
                "Pong expected value at {expected} but received it at {elapsed}",
            );
            *self.output = value;
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
  connection(from = "ping.output", to = "pong.input"),
  connection(from = "pong.output", to = "ping.input", after = "50 ms")
)]
struct Main {
    #[reactor(child(state = ping::PingState::default(), enclave))]
    ping: ping::Ping,
    #[reactor(child(state = pong::PongState::default(), enclave))]
    pong: pong::Pong,
}

#[test]
fn enclave_cycle() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_thread_names(true)
        .init();

    for _ in 0..100 {
        let config = runtime::Config::default()
            .with_fast_forward(true)
            .with_timeout(Duration::seconds(1));
        let (_, env) =
            boomerang_util::runner::build_and_test_reactor::<Main>("enclave_cycle", (), config)
                .unwrap();
    }
}
