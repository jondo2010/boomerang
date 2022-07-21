use boomerang::{builder::*, runtime, Reactor, boomerang_test_body};
use boomerang_util::{Timeout, TimeoutBuilder};

#[derive(Reactor)]
struct CountBuilder {
    #[reactor(timer(period = "1 sec"))]
    t: BuilderActionKey,
    #[reactor(output())]
    c: BuilderPortKey<u32>,
    #[reactor(child(state = "Timeout::new(runtime::Duration::from_secs(3))"))]
    _timeout: TimeoutBuilder,
    #[reactor(reaction(function = "Count::reaction_t",))]
    reaction_t: runtime::ReactionKey,
}

struct Count(u32);
impl Count {
    #[boomerang::reaction(reactor = "CountBuilder", triggers(timer = "t"))]
    fn reaction_t(
        &mut self,
        _ctx: &mut runtime::Context,
        #[reactor::port(effects, path = "c")] xyc: &mut runtime::Port<u32>,
    ) {
        self.0 += 1;
        assert!(xyc.is_none());
        *xyc.get_mut() = Some(dbg!(self.0));
    }
}

boomerang_test_body!(count, CountBuilder, Count(0));