use boomerang::{reaction, runtime, Reactor};

mod run;
pub use run::build_and_run_reactor;

#[derive(Reactor)]
pub struct TimeoutBuilder {
    #[reactor(reaction(function = "Timeout::reaction_startup"))]
    startup: runtime::ReactionKey,
}

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
