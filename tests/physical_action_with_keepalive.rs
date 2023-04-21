use boomerang::{
    builder::{BuilderReactionKey, Physical, TypedActionKey},
    runtime, Reactor,
};
use std::time::Duration;

#[derive(Reactor)]
#[reactor(state = "Main")]
struct MainBuilder {
    #[reactor(action(physical))]
    act: TypedActionKey<u32, Physical>,

    #[reactor(reaction(function = "Main::startup"))]
    reaction_startup: BuilderReactionKey,

    #[reactor(reaction(function = "Main::act"))]
    reaction_act: BuilderReactionKey,
}

#[derive(Clone)]
struct Main;
impl Main {
    #[boomerang::reaction(reactor = "MainBuilder", triggers(startup))]
    fn startup(
        &mut self,
        ctx: &mut runtime::Context,
        #[reactor::action(effects)] mut act: runtime::PhysicalActionRef<u32>,
    ) {
        let mut send_ctx = ctx.make_send_context();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            send_ctx.schedule_action(&mut act, Some(434), None);
        });
    }

    #[boomerang::reaction(reactor = "MainBuilder")]
    fn act(
        &mut self,
        ctx: &mut runtime::Context,
        #[reactor::action(triggers)] act: runtime::PhysicalActionRef<u32>,
    ) {
        let value = ctx.read_action_with(&act, |x| *x.unwrap());
        println!("---- Vu {} à {}", value, ctx.get_tag());

        let elapsed_time = ctx.get_elapsed_logical_time();
        assert!(elapsed_time >= Duration::from_millis(20));
        println!("success");
        ctx.schedule_shutdown(None);
    }
}

#[test_log::test]
#[cfg(not(feature = "federated"))]
fn physical_action_with_keepalive() {
    let _ = boomerang::runner::build_and_test_reactor::<MainBuilder>(
        "physical_action_with_keepalive",
        Main,
        true,
        true,
    )
    .unwrap();
}
