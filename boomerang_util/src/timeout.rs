//! Timeout reactor that schedules a shutdown after a specified duration.

use boomerang::prelude::*;

use std::time::Duration;

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Default, Debug)]
pub struct TimeoutDuration(Duration);

impl From<Duration> for TimeoutDuration {
    fn from(value: Duration) -> Self {
        Self(value)
    }
}

#[cfg(feature = "serde")]
runtime::register_type!(TimeoutDuration);

#[derive(Reactor)]
#[reactor(state = "TimeoutDuration", reaction = "ReactionStartup")]
pub struct Timeout;

#[derive(Reaction)]
#[reaction(triggers(startup), reactor = "Timeout")]
struct ReactionStartup;

impl runtime::Trigger<TimeoutDuration> for ReactionStartup {
    fn trigger(self, ctx: &mut runtime::Context, state: &mut TimeoutDuration) {
        ctx.schedule_shutdown(Some(state.0))
    }
}
