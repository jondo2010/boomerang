//! Tests is_present

use boomerang::prelude::*;

#[derive(Default)]
struct ActionIsPresentState {
    success: bool,
}

#[derive(Reactor, Clone)]
#[reactor(
    state = "ActionIsPresentState",
    reaction = "ReactionStartup",
    reaction = "ReactionShutdown"
)]
struct ActionIsPresent {
    #[reactor(action())]
    a: TypedActionKey,
}

#[derive(Reaction)]
#[reaction(reactor = "ActionIsPresent", triggers(startup))]
struct ReactionStartup<'a> {
    #[reaction(triggers)]
    a: runtime::ActionRef<'a>,
}

impl runtime::Trigger<ActionIsPresentState> for ReactionStartup<'_> {
    fn trigger(mut self, ctx: &mut runtime::Context, state: &mut ActionIsPresentState) {
        if !self.a.is_present(ctx) {
            assert!(!state.success, "Unexpected");
            println!("Hello startup!");
            ctx.schedule_action(&mut self.a, (), Some(Duration::nanoseconds(1)));
        } else {
            println!("Hello a!");
            state.success = true;
        }
    }
}

#[derive(Reaction)]
#[reaction(reactor = "ActionIsPresent", triggers(shutdown))]
struct ReactionShutdown;

impl runtime::Trigger<ActionIsPresentState> for ReactionShutdown {
    fn trigger(self, _ctx: &mut runtime::Context, state: &mut ActionIsPresentState) {
        assert!(state.success, "Failed to print 'Hello World!'");
    }
}

#[test]
fn action_is_present() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default().with_fast_forward(true);
    let (_, envs) = boomerang_util::runner::build_and_test_reactor::<ActionIsPresent>(
        "action_is_present",
        ActionIsPresentState::default(),
        config,
    )
    .unwrap();

    let state = envs[0]
        .find_reactor_by_name("action_is_present")
        .and_then(|reactor| reactor.get_state::<ActionIsPresentState>())
        .unwrap();
    assert!(
        state.success,
        "ReactionStartup did not trigger successfully"
    );
}
