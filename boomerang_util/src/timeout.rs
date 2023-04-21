//! A reactor that schedules a shutdown after a given timeout.

use std::time::Duration;

use boomerang::{builder::BuilderReactionKey, reaction, runtime, Reactor};

#[derive(Reactor)]
#[reactor(state = "Timeout")]
pub struct TimeoutBuilder {
    #[reactor(reaction(function = "Timeout::reaction_startup"))]
    startup: BuilderReactionKey,
}

#[derive(Debug, Clone)]
pub struct Timeout {
    timeout: Duration,
}

impl Timeout {
    pub fn new(timeout: Duration) -> Self {
        Self { timeout }
    }

    #[reaction(reactor = "TimeoutBuilder", triggers(startup))]
    fn reaction_startup(&mut self, ctx: &mut runtime::Context) {
        ctx.schedule_shutdown(Some(self.timeout))
    }
}
