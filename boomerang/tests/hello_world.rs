use boomerang::{builder::BuilderReactionKey, runtime, Reactor};

#[derive(Reactor)]
#[reactor(state = "HelloWorld2")]
struct HelloWorld2Builder {
    #[reactor(reaction(function = "HelloWorld2::reaction_startup"))]
    reaction_startup: BuilderReactionKey,
    #[reactor(reaction(function = "HelloWorld2::reaction_shutdown"))]
    reaction_shutdown: BuilderReactionKey,
}
#[derive(Clone)]
struct HelloWorld2 {
    success: bool,
}
impl HelloWorld2 {
    #[boomerang::reaction(reactor = "HelloWorld2Builder", triggers(startup))]
    fn reaction_startup(&mut self, _ctx: &runtime::Context) {
        println!("Hello World.");
        self.success = true;
    }

    #[boomerang::reaction(reactor = "HelloWorld2Builder", triggers(shutdown))]
    fn reaction_shutdown(&mut self, _ctx: &runtime::Context) {
        println!("Shutdown invoked.");
        assert!(self.success, "ERROR: startup reaction not executed.");
    }
}

#[derive(Reactor)]
#[reactor(state = "()")]
struct HelloWorldBuilder {
    #[reactor(child(rename = "a", state = "HelloWorld2{success: false}"))]
    _a: HelloWorld2Builder,
}

#[test_log::test]
#[cfg(not(feature = "federated"))]
fn hello_world() {
    let _ = boomerang_util::run::build_and_test_reactor::<HelloWorldBuilder>(
        "hello_world",
        (),
        true,
        false,
    )
    .unwrap();
}
