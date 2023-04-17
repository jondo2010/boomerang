// Test logical action with delay.

use std::time::Duration;

use boomerang::{
    builder::{BuilderReactionKey, TypedActionKey},
    runtime, Reactor,
};

#[derive(Reactor)]
#[reactor(state = "ActionValues")]
struct ActionValuesBuilder {
    #[reactor(action(min_delay = "100 msec"))]
    act: TypedActionKey<i32>,
    #[reactor(reaction(function = "ActionValues::reaction_startup"))]
    reaction_startup: BuilderReactionKey,
    #[reactor(reaction(function = "ActionValues::reaction_act"))]
    reaction_act: BuilderReactionKey,
    #[reactor(reaction(function = "ActionValues::reaction_shutdown"))]
    reaction_shutdown: BuilderReactionKey,
}

#[derive(Clone)]
struct ActionValues {
    r1done: bool,
    r2done: bool,
}

impl ActionValues {
    #[boomerang::reaction(reactor = "ActionValuesBuilder", triggers(startup))]
    fn reaction_startup(
        &mut self,
        ctx: &mut runtime::Context,
        #[reactor::action(effects)] mut act: runtime::ActionRef<i32>,
    ) {
        // scheduled in 100 ms
        ctx.schedule_action(&mut act, Some(100), None);
        // scheduled in 150 ms, value is overwritten
        ctx.schedule_action(&mut act, Some(-100), Some(Duration::from_millis(50)));
    }

    #[boomerang::reaction(reactor = "ActionValuesBuilder")]
    fn reaction_act(
        &mut self,
        ctx: &mut runtime::Context,
        #[reactor::action(triggers)] act: runtime::ActionRef<i32>,
    ) {
        let elapsed = ctx.get_elapsed_logical_time();
        let value = ctx.get_action(&act);

        println!("[@{elapsed:?} action transmitted: {value:?}]");

        if elapsed.as_millis() == 100 {
            assert_eq!(value, Some(100), "ERROR: Expected action value to be 100");
            self.r1done = true;
        } else {
            if elapsed.as_millis() != 150 {
                panic!("ERROR: Unexpected reaction invocation at {elapsed:?}");
            }
            assert_eq!(value, Some(-100), "ERROR: Expected action value to be -100");
            self.r2done = true;
        }
    }

    #[boomerang::reaction(reactor = "ActionValuesBuilder", triggers(shutdown))]
    fn reaction_shutdown(&mut self, _ctx: &mut runtime::Context) {
        assert!(
            self.r1done && self.r2done,
            "ERROR: Expected 2 reaction invocations\n"
        );
    }
}

#[test_log::test]
fn action_values() {
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
