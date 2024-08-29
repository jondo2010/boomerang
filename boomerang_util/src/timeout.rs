//! Timeout reactor that schedules a shutdown after a specified duration.

use boomerang::{
    builder::{self, Trigger},
    runtime, Reaction, Reactor,
};

#[derive(Reactor, Clone)]
#[reactor(state = runtime::Duration)]
pub struct Timeout {
    startup: builder::TypedReactionKey<ReactionStartup>,
}

#[derive(Reaction)]
#[reaction(triggers(startup))]
struct ReactionStartup;

impl Trigger for ReactionStartup {
    type Reactor = Timeout;

    fn trigger(&mut self, ctx: &mut runtime::Context, state: &mut runtime::Duration) {
        ctx.schedule_shutdown(Some(*state))
    }
}
