use boomerang::prelude::*;

#[derive(Reactor)]
#[reactor(state = "bool", reaction = "ReactionStartup", reaction = "ReactionAct")]
struct MainBuilder {
    act: TypedActionKey<u32, Physical>,
}

#[derive(Reaction)]
#[reaction(reactor = "MainBuilder", triggers(startup))]
struct ReactionStartup {
    act: runtime::AsyncActionRef<u32>,
}

impl runtime::Trigger<bool> for ReactionStartup {
    fn trigger(self, ctx: &mut runtime::Context, _state: &mut bool) {
        let send_ctx = ctx.make_send_context();
        let act = self.act.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(20));
            act.schedule(&send_ctx, 434, None);
        });
    }
}

#[derive(Reaction)]
#[reaction(reactor = "MainBuilder")]
struct ReactionAct<'a> {
    #[reaction(triggers)]
    act: runtime::ActionRef<'a, u32>,
}

impl runtime::Trigger<bool> for ReactionAct<'_> {
    fn trigger(mut self, ctx: &mut runtime::Context, _state: &mut bool) {
        let value = self.act.get_value(ctx).unwrap();
        println!("---- Vu {} à {}", value, ctx.get_tag());

        let elapsed_time = ctx.get_elapsed_logical_time();
        assert!(elapsed_time >= Duration::milliseconds(20));
        println!("success");
        *_state = true;
        ctx.schedule_shutdown(None);
    }
}

#[test]
fn physical_action_with_keepalive() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default()
        .with_fast_forward(true)
        .with_keep_alive(true);
    let (_, sched) = boomerang_util::runner::build_and_test_reactor::<MainBuilder>(
        "physical_action_with_keepalive",
        false,
        config,
    )
    .unwrap();

    let env = sched.into_env();
    let reactor = env
        .find_reactor_by_name("physical_action_with_keepalive")
        .unwrap();
    assert_eq!(reactor.get_state(), Some(&true));
}
