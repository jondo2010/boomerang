//! Timeout reactor that schedules a shutdown after a specified duration.

use boomerang::{
    builder::{self, Trigger},
    runtime, Reaction, Reactor,
};

#[derive(Reactor, Clone)]
#[reactor(state = "runtime::Duration", reaction = "ReactionStartup")]
pub struct Timeout;

#[derive(Reaction)]
#[reaction(triggers(startup), reactor = "Timeout")]
struct ReactionStartup;

impl Trigger<Timeout> for ReactionStartup {
    fn trigger(self, ctx: &mut runtime::Context, state: &mut runtime::Duration) {
        ctx.schedule_shutdown(Some(*state))
    }
}
