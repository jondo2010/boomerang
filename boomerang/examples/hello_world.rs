use boomerang::{runtime, Reactor, run};

#[derive(Reactor)]
#[reactor(state = "HelloWorld2")]
struct HelloWorld2Builder {
    #[reactor(reaction(function = "HelloWorld2::reaction_startup"))]
    reaction_startup: runtime::ReactionKey,
    #[reactor(reaction(function = "HelloWorld2::reaction_shutdown"))]
    reaction_shutdown: runtime::ReactionKey,
}
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

fn main() {
    tracing_subscriber::fmt::init();
    let _ = run::build_and_run_reactor::<HelloWorldBuilder>("hello_world", ()).unwrap();
}
