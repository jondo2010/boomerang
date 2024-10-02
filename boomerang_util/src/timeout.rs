//! Timeout reactor that schedules a shutdown after a specified duration.

use boomerang::prelude::*;

use std::time::Duration;

#[derive(Reactor)]
#[reactor(state = "Duration", reaction = "ReactionStartup")]
pub struct Timeout;

#[derive(Reaction)]
#[reaction(triggers(startup), reactor = "Timeout")]
struct ReactionStartup;

impl Trigger<Timeout> for ReactionStartup {
    fn trigger(self, ctx: &mut runtime::Context, state: &mut Duration) {
        ctx.schedule_shutdown(Some(*state))
    }
}
