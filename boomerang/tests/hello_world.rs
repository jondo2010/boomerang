use boomerang::prelude::*;

#[derive(Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
struct State {
    success: bool,
}

#[derive(Reactor)]
#[reactor(
    state = "State",
    reaction = "HelloWorld2ReactionStartup",
    reaction = "HelloWorld2ReactionShutdown"
)]
struct HelloWorld2;

#[derive(Reaction)]
#[reaction(reactor = "HelloWorld2", triggers(startup))]
struct HelloWorld2ReactionStartup;

impl runtime::Trigger<State> for HelloWorld2ReactionStartup {
    fn trigger(self, _ctx: &mut runtime::Context, state: &mut State) {
        println!("Hello World.");
        state.success = true;
    }
}

#[derive(Reaction)]
#[reaction(reactor = "HelloWorld2", triggers(shutdown))]
struct HelloWorld2ReactionShutdown;

impl runtime::Trigger<State> for HelloWorld2ReactionShutdown {
    fn trigger(self, _ctx: &mut runtime::Context, state: &mut State) {
        println!("Shutdown invoked.");
        state.success = false;
    }
}

#[derive(Reactor)]
#[reactor(state = "()")]
struct HelloWorld {
    #[reactor(child(state = State{success: false}))]
    _a: HelloWorld2,
}

#[test]
fn hello_world() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default().with_fast_forward(true);
    let _ = boomerang_util::runner::build_and_test_reactor::<HelloWorld>("hello_world", (), config)
        .unwrap();
}
