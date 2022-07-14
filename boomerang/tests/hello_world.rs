use boomerang::{runtime, Reactor};
use std::convert::TryInto;

#[derive(Reactor)]
#[reactor(
    reaction(function = "HelloWorld2::reaction_startup", triggers(startup)),
    reaction(function = "HelloWorld2::reaction_shutdown", triggers(shutdown))
)]
struct HelloWorld2 {
    success: bool,
}
impl HelloWorld2 {
    fn reaction_startup<S: runtime::SchedulerPoint>(
        &mut self,
        _sched: &S,
        _inputs: &HelloWorld2Inputs,
        _outputs: &HelloWorld2Outputs,
        _actions: &HelloWorld2Actions,
    ) {
        println!("Hello World.");
        self.success = true;
    }

    fn reaction_shutdown<S: runtime::SchedulerPoint>(
        &mut self,
        _sched: &S,
        _inputs: &HelloWorld2Inputs,
        _outputs: &HelloWorld2Outputs,
        _actions: &HelloWorld2Actions,
    ) {
        println!("Shutdown invoked.");
        assert!(self.success, "ERROR: startup reaction not executed.");
    }
}

#[derive(Reactor)]
#[reactor(child(reactor = "HelloWorld2{success: false}", name = "a"))]
struct HelloWorld {}

#[test]
fn test() {
    use boomerang::{builder::*, runtime};
    let mut env_builder = EnvBuilder::new();

    let _ = HelloWorld {}.build("a", &mut env_builder, None).unwrap();

    let env: runtime::Env<_> = env_builder.try_into().unwrap();
    let mut sched = runtime::Scheduler::new(env);
    sched.start().unwrap();
}
