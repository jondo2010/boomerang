use boomerang::prelude::*;
use runtime::Duration;

#[derive(Reactor)]
#[reactor(state = "()", reaction = "ReactionStartup", reaction = "ReactionAct")]
struct MainBuilder {
    act: TypedActionKey<u32, Physical>,
}

#[derive(Reaction)]
#[reaction(reactor = "MainBuilder", triggers(startup))]
struct ReactionStartup {
    act: runtime::PhysicalActionRef<u32>,
}

impl Trigger<MainBuilder> for ReactionStartup {
    fn trigger(self, ctx: &mut runtime::Context, _state: &mut ()) {
        let mut send_ctx = ctx.make_send_context();
        let mut act = self.act.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            send_ctx.schedule_action(&mut act, Some(434), None);
        });
    }
}

#[derive(Reaction)]
#[reaction(reactor = "MainBuilder")]
struct ReactionAct {
    #[reaction(triggers)]
    act: runtime::PhysicalActionRef<u32>,
}

impl Trigger<MainBuilder> for ReactionAct {
    fn trigger(mut self, ctx: &mut runtime::Context, _state: &mut ()) {
        let value = ctx
            .get_action_with(&mut self.act, |value| value.cloned())
            .unwrap();
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
    let config = runtime::Config::default()
        .with_fast_forward(true)
        .with_keep_alive(true);
    let _ = boomerang_util::runner::build_and_test_reactor::<MainBuilder>(
        "physical_action_with_keepalive",
        (),
        config,
    )
    .unwrap();
}
