use boomerang::{builder::*, runtime, Reaction, Reactor};
use boomerang_util::timeout;

#[derive(Reactor, Clone)]
#[reactor(
    state = u32,
    reaction=ReactionT,
    reaction=ReactionShutdown,
)]
struct Count {
    #[reactor(timer(period = "1 msec"))]
    t: TimerActionKey,
    c: TypedPortKey<u32, Output>,
    #[reactor(child = runtime::Duration::from_secs(1))]
    _timeout: timeout::Timeout,
    reaction_t: TypedReactionKey<ReactionT<'static>>,
    reaction_shutdown: TypedReactionKey<ReactionShutdown>,
}

#[derive(Reaction)]
#[reaction(triggers(action = "t"))]
struct ReactionT<'a> {
    #[reaction(path = "c")]
    xyc: runtime::OutputRef<'a, u32>,
}

impl Trigger for ReactionT<'_> {
    type Reactor = Count;

    fn trigger(
        &mut self,
        _ctx: &mut runtime::Context,
        state: &mut <Self::Reactor as Reactor>::State,
    ) {
        *state += 1;
        assert!(self.xyc.is_none());
        *self.xyc = Some(*state);
    }
}

#[derive(Reaction)]
#[reaction(triggers(shutdown))]
struct ReactionShutdown;

impl Trigger for ReactionShutdown {
    type Reactor = Count;

    fn trigger(
        &mut self,
        _ctx: &mut runtime::Context,
        state: &mut <Self::Reactor as Reactor>::State,
    ) {
        assert_eq!(*state, 1e3 as u32, "expected 1e3, got {state}");
        println!("ok");
    }
}

#[test]
fn count() {
    tracing_subscriber::fmt::init();
    let (_, env) =
        boomerang_util::run::build_and_test_reactor::<Count>("count", 0, true, false).unwrap();

    let count = env
        .get_reactor_by_name("count")
        .and_then(|r| r.get_state::<u32>())
        .unwrap();
    assert_eq!(*count, 1e3 as u32);
}
