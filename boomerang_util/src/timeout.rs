//! Timeout reactor that schedules a shutdown after a specified duration.

use boomerang::{builder, reaction, runtime, Reactor};

#[derive(Reactor, Clone)]
#[reactor(state = "Timeout")]
pub struct TimeoutBuilder {
    #[reactor(reaction(function = "Timeout::reaction_startup"))]
    startup: builder::BuilderReactionKey,
}

#[derive(Debug)]
pub struct Timeout {
    timeout: runtime::Duration,
}

impl Timeout {
    pub fn new(timeout: runtime::Duration) -> Self {
        Self { timeout }
    }

    #[reaction(reactor = "TimeoutBuilder", triggers(startup))]
    fn reaction_startup(&mut self, ctx: &mut runtime::Context) {
        ctx.schedule_shutdown(Some(self.timeout))
    }
}
