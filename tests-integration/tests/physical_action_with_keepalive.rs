use boomerang::{builder::prelude::*, runtime, Reaction, Reactor};
use runtime::Duration;

#[derive(Clone, Reactor)]
#[reactor(state = ())]
struct MainBuilder {
    #[reactor(action(physical))]
    act: TypedActionKey<u32, Physical>,

    reaction_startup: TypedReactionKey<ReactionStartup>,
    reaction_act: TypedReactionKey<ReactionAct>,
}

#[derive(Reaction)]
#[reaction(triggers(startup))]
struct ReactionStartup {
    act: runtime::PhysicalActionRef<u32>,
}

impl Trigger for ReactionStartup {
    type Reactor = MainBuilder;

    fn trigger(&mut self, ctx: &mut runtime::Context, state: &mut ()) {
        let mut send_ctx = ctx.make_send_context();
        let mut act = self.act.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            send_ctx.schedule_action(&mut act, Some(434), None);
        });
    }
}

#[derive(Reaction)]
struct ReactionAct {
    #[reaction(triggers)]
    act: runtime::PhysicalActionRef<u32>,
}

impl Trigger for ReactionAct {
    type Reactor = MainBuilder;

    fn trigger(&mut self, ctx: &mut runtime::Context, state: &mut ()) {
        let value = ctx.get_action(&mut self.act).unwrap();
        println!("---- Vu {} à {}", value, ctx.get_tag());

        let elapsed_time = ctx.get_elapsed_logical_time();
        assert!(elapsed_time >= Duration::from_millis(20));
        println!("success");
        ctx.schedule_shutdown(None);
    }
}

#[test]
fn physical_action_with_keepalive() {
    tracing_subscriber::fmt::init();
    let _ = boomerang_util::run::build_and_test_reactor::<MainBuilder>(
        "physical_action_with_keepalive",
        (),
        true,
        true,
    )
    .unwrap();
}
