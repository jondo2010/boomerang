use boomerang::{runtime, Reactor};
use std::convert::TryInto;

#[derive(Reactor)]
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
struct HelloWorldBuilder {
    #[reactor(child(rename = "a", state = "HelloWorld2{success: false}"))]
    _a: HelloWorld2Builder,
}

#[test]
fn test() {
    use boomerang::{builder::*, runtime};
    let mut env_builder = EnvBuilder::new();

    let _ = HelloWorldBuilder::build("a", (), None, &mut env_builder);

    let (env, dep_info) = env_builder.try_into().unwrap();

    runtime::check_consistency(&env, &dep_info);
    runtime::debug_info(&env, &dep_info);

    let sched = runtime::Scheduler::new(env, dep_info, false);
    sched.event_loop();
}
