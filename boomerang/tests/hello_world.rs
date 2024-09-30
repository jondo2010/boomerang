use boomerang::prelude::*;

struct State {
    success: bool,
}

#[derive(Clone, Reactor)]
#[reactor(
    state = "State",
    reaction = "HelloWorld2ReactionStartup",
    reaction = "HelloWorld2ReactionShutdown"
)]
struct HelloWorld2;

#[derive(Reaction)]
#[reaction(reactor = "HelloWorld2", triggers(startup))]
struct HelloWorld2ReactionStartup;

impl Trigger<HelloWorld2> for HelloWorld2ReactionStartup {
    fn trigger(self, _ctx: &mut runtime::Context, state: &mut State) {
        println!("Hello World.");
        state.success = true;
    }
}

#[derive(Reaction)]
#[reaction(reactor = "HelloWorld2", triggers(shutdown))]
struct HelloWorld2ReactionShutdown;

impl Trigger<HelloWorld2> for HelloWorld2ReactionShutdown {
    fn trigger(self, _ctx: &mut runtime::Context, state: &mut State) {
        println!("Shutdown invoked.");
        state.success = false;
    }
}

#[derive(Clone, Reactor)]
#[reactor(state = ())]
struct HelloWorld {
    #[reactor(child = State{success: false})]
    _a: HelloWorld2,
}

#[test]
fn hello_world() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default().with_fast_forward(true);
    let _ = boomerang_util::runner::build_and_test_reactor::<HelloWorld>("hello_world", (), config)
        .unwrap();
}
