// Test logical action with delay.

use boomerang::{
    builder::{Trigger, TypedActionKey, TypedReactionKey},
    runtime, Reaction, Reactor,
};

#[derive(Clone, Reactor)]
#[reactor(state = ActionValues)]
struct ActionValuesBuilder {
    #[reactor(action(min_delay = "100 msec"))]
    act: TypedActionKey<i32>,
    reaction_startup: TypedReactionKey<ReactionStartup<'static>>,
    reaction_act: TypedReactionKey<ReactionAct<'static>>,
    reaction_shutdown: TypedReactionKey<ReactionShutdown>,
}

struct ActionValues {
    r1done: bool,
    r2done: bool,
}

#[derive(Reaction)]
#[reaction(triggers(startup))]
struct ReactionStartup<'a> {
    act: runtime::ActionRef<'a, i32>,
}

impl<'a> Trigger for ReactionStartup<'a> {
    type Reactor = ActionValuesBuilder;

    fn trigger(&mut self, ctx: &mut runtime::Context, _state: &mut ActionValues) {
        // scheduled in 100 ms
        ctx.schedule_action(&mut self.act, Some(100), None);
        // scheduled in 150 ms, value is overwritten
        ctx.schedule_action(
            &mut self.act,
            Some(-100),
            Some(runtime::Duration::from_millis(50)),
        );
    }
}

#[derive(Reaction)]
struct ReactionAct<'a> {
    #[reaction(triggers)]
    act: runtime::ActionRef<'a, i32>,
}

impl<'a> Trigger for ReactionAct<'a> {
    type Reactor = ActionValuesBuilder;

    fn trigger(&mut self, ctx: &mut runtime::Context, state: &mut ActionValues) {
        let elapsed = ctx.get_elapsed_logical_time();
        let value = ctx.get_action(&self.act);

        println!("[@{elapsed:?} action transmitted: {value:?}]");

        if elapsed.as_millis() == 100 {
            assert_eq!(value, Some(100), "ERROR: Expected action value to be 100");
            state.r1done = true;
        } else {
            if elapsed.as_millis() != 150 {
                panic!("ERROR: Unexpected reaction invocation at {elapsed:?}");
            }
            assert_eq!(value, Some(-100), "ERROR: Expected action value to be -100");
            state.r2done = true;
        }
    }
}

#[derive(Reaction)]
#[reaction(triggers(shutdown))]
struct ReactionShutdown;

impl Trigger for ReactionShutdown {
    type Reactor = ActionValuesBuilder;

    fn trigger(&mut self, _ctx: &mut runtime::Context, state: &mut ActionValues) {
        assert!(
            state.r1done && state.r2done,
            "ERROR: Expected 2 reaction invocations\n"
        );
        println!("Ok.");
    }
}

#[test]
fn action_values() {
    tracing_subscriber::fmt::init();
    let _ = boomerang_util::run::build_and_test_reactor::<ActionValuesBuilder>(
        "action_values",
        ActionValues {
            r1done: false,
            r2done: false,
        },
        true,
        false,
    )
    .unwrap();
}
