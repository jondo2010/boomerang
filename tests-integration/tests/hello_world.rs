use boomerang::{builder::prelude::*, runtime, Reaction, Reactor};

struct State {
    success: bool,
}

#[derive(Clone, Reactor)]
#[reactor(state = State)]
struct HelloWorld2 {
    reaction_startup: TypedReactionKey<HelloWorld2ReactionStartup>,
    reaction_shutdown: TypedReactionKey<HelloWorld2ReactionShutdown>,
}

#[derive(Reaction)]
#[reaction(triggers(startup))]
struct HelloWorld2ReactionStartup;

impl Trigger for HelloWorld2ReactionStartup {
    type Reactor = HelloWorld2;
    fn trigger(&mut self, _ctx: &mut runtime::Context, state: &mut State) {
        println!("Hello World.");
        state.success = true;
    }
}

#[derive(Reaction)]
#[reaction(triggers(shutdown))]
struct HelloWorld2ReactionShutdown;

impl Trigger for HelloWorld2ReactionShutdown {
    type Reactor = HelloWorld2;
    fn trigger(&mut self, _ctx: &mut runtime::Context, state: &mut State) {
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
    let _ =
        boomerang_util::run::build_and_test_reactor::<HelloWorld>("hello_world", (), true, false)
            .unwrap();
}
