use boomerang::{
    builder::*,
    runtime::{self},
    Reaction, Reactor,
};
use boomerang_util::timeout;

trait CountData:
    Copy + runtime::PortData + std::ops::AddAssign<i32> + std::cmp::PartialEq<i32>
{
}

impl CountData for i32 {}

#[derive(Reactor, Clone)]
#[reactor(state = "T", reaction = "ReactionT<T>", reaction = "ReactionShutdown")]
struct Count<T: CountData> {
    #[reactor(timer(period = "1 msec"))]
    t: TimerActionKey,
    c: TypedPortKey<T, Output>,
    #[reactor(child = runtime::Duration::from_secs(1))]
    _timeout: timeout::Timeout,
}

#[derive(Reaction)]
#[reaction(
    reactor = "Count::<T>",
    //bound = "T: runtime::PortData",
    triggers(action = "t")
)]
struct ReactionT<'a, T: CountData> {
    #[reaction(path = "c")]
    xyc: runtime::OutputRef<'a, T>,
}

impl<T: CountData> Trigger<Count<T>> for ReactionT<'_, T> {
    fn trigger(mut self, _ctx: &mut runtime::Context, state: &mut <Count<T> as Reactor>::State) {
        *state += 1;
        assert!(self.xyc.is_none());
        *self.xyc = Some(*state);
    }
}

#[derive(Reaction)]
#[reaction(reactor = "Count::<T>", bound = "T: CountData", triggers(shutdown))]
struct ReactionShutdown;

impl<T: CountData> Trigger<Count<T>> for ReactionShutdown {
    fn trigger(self, _ctx: &mut runtime::Context, state: &mut <Count<T> as Reactor>::State) {
        assert_eq!(*state, 1e3 as i32, "expected 1e3, got {state:?}");
        println!("ok");
    }
}

#[test]
fn count() {
    tracing_subscriber::fmt::init();
    //let (_, env) = boomerang_util::run::build_and_test_reactor::<Count<u32>>("count", 0, true, false).unwrap();

    let mut env_builder = EnvBuilder::new();
    let reactor = <Count<i32> as boomerang::builder::Reactor>::build(
        "count",
        0,
        None,
        None,
        &mut env_builder,
    )
    .unwrap();
    let (mut env, triggers, _) = env_builder.into_runtime_parts().unwrap();
    let mut sched = runtime::Scheduler::new(&mut env, triggers, true, false);
    sched.event_loop();

    let count = env
        .get_reactor_by_name("count")
        .and_then(|r| r.get_state::<i32>())
        .unwrap();
    assert_eq!(*count, 1e3 as i32);
}
