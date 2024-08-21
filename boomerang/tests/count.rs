use boomerang::{builder::*, runtime, Reactor};
use boomerang_util::timeout::{Timeout, TimeoutBuilder};

struct Count(u32);

#[derive(Reactor)]
#[reactor(state = "Count")]
struct CountBuilder {
    #[reactor(timer(period = "1 usec"))]
    t: TypedActionKey<()>,
    #[reactor(output())]
    c: TypedPortKey<u32>,
    #[reactor(child(state = "Timeout::new(runtime::Duration::from_secs(1))"))]
    _timeout: TimeoutBuilder,
    #[reactor(reaction(function = "Count::reaction_t",))]
    reaction_t: BuilderReactionKey,
    #[reactor(reaction(function = "Count::shutdown_t"))]
    reaction_shutdown_t: BuilderReactionKey,
}

impl Count {
    #[boomerang::reaction(reactor = "CountBuilder", triggers(action = "t"))]
    fn reaction_t(
        &mut self,
        _ctx: &mut runtime::Context,
        #[reactor::port(effects, path = "c")] xyc: &mut runtime::Port<u32>,
    ) {
        self.0 += 1;
        assert!(xyc.is_none());
        *xyc.get_mut() = Some(self.0);
    }

    #[boomerang::reaction(reactor = "CountBuilder", triggers(shutdown))]
    fn shutdown_t(&mut self, _ctx: &mut runtime::Context) {
        assert_eq!(self.0, 1e6 as u32 - 1, "expected 1e6, got {}", self.0);
        println!("ok");
    }
}

#[derive(boomerang_derive2::Reaction)]
#[reaction(triggers(startup))]
struct ReactionT<'a> {
    #[reaction(triggers)]
    t: &'a runtime::ActionRef<'a>,
    #[reaction(effects, path = "c")]
    xyc: &'a mut runtime::Port<u32>,
}

impl Trigger for ReactionT<'_> {
    type BuilderReactor = CountBuilder;

    fn trigger(
        self,
        _ctx: &mut runtime::Context,
        state: &mut <Self::BuilderReactor as Reactor>::State,
    ) {
        state.0 += 1;
        assert!(self.xyc.is_none());
        *self.xyc.get_mut() = Some(state.0);
    }
}

#[test]
fn count() {
    tracing_subscriber::fmt::init();
    let (_, env) =
        boomerang_util::run::build_and_test_reactor::<CountBuilder>("count", Count(0), true, false)
            .unwrap();
    let count = env
        .get_reactor_by_name("count")
        .and_then(|r| r.get_state::<Count>())
        .unwrap();
    assert_eq!(count.0, 1e6 as u32 - 1);
}
