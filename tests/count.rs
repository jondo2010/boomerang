use boomerang::{builder::*, runtime, Reactor};
use boomerang_util::timeout::{Timeout, TimeoutBuilder};
use std::time::Duration;

#[derive(Reactor)]
#[reactor(state = "Count")]
struct CountBuilder {
    #[reactor(timer(period = "1 sec"))]
    t: TypedActionKey<()>,
    #[reactor(output())]
    c: TypedPortKey<u32>,
    #[reactor(child(state = "Timeout::new(Duration::from_secs(3))"))]
    _timeout: TimeoutBuilder,
    #[reactor(reaction(function = "Count::reaction_t",))]
    reaction_t: BuilderReactionKey,
}

#[derive(Clone)]
struct Count(u32);
impl Count {
    #[boomerang::reaction(reactor = "CountBuilder", triggers(action = "t"))]
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

#[test_log::test]
#[cfg(not(feature = "federated"))]
fn count() {
    let _ =
        boomerang::runner::build_and_test_reactor::<CountBuilder>("count", Count(0), true, false)
            .unwrap();
}
