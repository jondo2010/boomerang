use boomerang::{builder::*, runtime, Reactor};
use boomerang_util::timeout::{Timeout, TimeoutBuilder};

#[derive(Reactor)]
#[reactor(state = "Count")]
struct CountBuilder {
    #[reactor(timer(period = "1 sec"))]
    t: TypedActionKey<()>,
    #[reactor(output())]
    c: TypedPortKey<u32>,
    #[reactor(child(state = "Timeout::new(runtime::Duration::from_secs(3))"))]
    _timeout: TimeoutBuilder,
    #[reactor(reaction(function = "Count::reaction_t",))]
    reaction_t: BuilderReactionKey,
}

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

#[derive(boomerang_derive2::Reaction)]
#[reaction(reactor = "CountBuilder", triggers(startup))]
struct ReactionT<'a> {
    #[reaction(triggers)]
    t: &'a runtime::ActionRef<'a>,
    #[reaction(effects, path = "c")]
    xyc: &'a mut runtime::Port<u32>,
}

#[test]
fn count() {
    tracing_subscriber::fmt::init();
    let _ =
        boomerang_util::run::build_and_test_reactor::<CountBuilder>("count", Count(0), true, false)
            .unwrap();
}
