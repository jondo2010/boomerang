//! Test logical action with delay.

use boomerang::prelude::*;

#[derive(Default, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
struct State {
    r1done: bool,
    r2done: bool,
}

#[derive(Clone, Reactor)]
#[reactor(
    state = "State",
    reaction = "ReactionStartup",
    reaction = "ReactionAct",
    reaction = "ReactionShutdown"
)]
struct ActionValues {
    #[reactor(action(min_delay = "100 msec"))]
    act: TypedActionKey<i32>,
}

#[derive(Reaction)]
#[reaction(reactor = "ActionValues", triggers(startup))]
struct ReactionStartup<'a> {
    act: runtime::ActionRef<'a, i32>,
}

impl<'a> runtime::Trigger<State> for ReactionStartup<'a> {
    fn trigger(mut self, ctx: &mut runtime::Context, _state: &mut State) {
        // scheduled in 100 ms
        ctx.schedule_action(&mut self.act, 100, None);
        // scheduled in 150 ms, value is overwritten
        ctx.schedule_action(&mut self.act, -100, Some(Duration::milliseconds(50)));
    }
}

#[derive(Reaction)]
#[reaction(reactor = "ActionValues")]
struct ReactionAct<'a> {
    #[reaction(triggers)]
    act: runtime::ActionRef<'a, i32>,
}

impl<'a> runtime::Trigger<State> for ReactionAct<'a> {
    fn trigger(mut self, ctx: &mut runtime::Context, state: &mut State) {
        let elapsed = ctx.get_elapsed_logical_time();
        let value = ctx.get_action_value(&mut self.act);

        println!("[@{elapsed:?} action transmitted: {value:?}]");

        if elapsed.whole_milliseconds() == 100 {
            assert_eq!(value, Some(&100), "ERROR: Expected action value to be 100");
            state.r1done = true;
        } else {
            if elapsed.whole_milliseconds() != 150 {
                panic!("ERROR: Unexpected reaction invocation at {elapsed:?}");
            }
            assert_eq!(
                value,
                Some(&-100),
                "ERROR: Expected action value to be -100"
            );
            state.r2done = true;
        }
    }
}

#[derive(Reaction)]
#[reaction(reactor = "ActionValues", triggers(shutdown))]
struct ReactionShutdown;

impl runtime::Trigger<State> for ReactionShutdown {
    fn trigger(self, _ctx: &mut runtime::Context, state: &mut State) {
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
    let config = runtime::Config::default().with_fast_forward(true);
    let _ = boomerang_util::runner::build_and_test_reactor::<ActionValues>(
        "action_values",
        State {
            r1done: false,
            r2done: false,
        },
        config,
    )
    .unwrap();
}
