use boomerang::prelude::*;

use std::fmt::Debug;

trait CountData:
    Debug + Copy + runtime::ReactorData + std::ops::AddAssign<i32> + std::cmp::PartialEq<i32>
{
}

impl CountData for i32 {}

#[derive(Reactor)]
#[reactor(state = "T", reaction = "ReactionT<T>", reaction = "ReactionShutdown")]
struct Count<T: CountData> {
    #[reactor(timer(period = "1 sec"))]
    t: TimerActionKey,
    c: TypedPortKey<T, Output>,
}

#[derive(Reaction)]
#[reaction(reactor = "Count::<T>", triggers(action = "t"))]
struct ReactionT<'a, T: CountData> {
    #[reaction(path = "c")]
    xyc: runtime::OutputRef<'a, T>,
}

impl<T: CountData> runtime::Trigger<T> for ReactionT<'_, T> {
    fn trigger(mut self, _ctx: &mut runtime::Context, state: &mut T) {
        *state += 1;
        assert!(self.xyc.is_none());
        *self.xyc = Some(*state);
    }
}

#[derive(Reaction)]
#[reaction(reactor = "Count::<T>", bound = "T: CountData", triggers(shutdown))]
struct ReactionShutdown;

impl<T: CountData> runtime::Trigger<T> for ReactionShutdown {
    fn trigger(self, _ctx: &mut runtime::Context, state: &mut T) {
        assert_eq!(*state, 4, "expected 4, got {state:?}");
        println!("ok");
    }
}

#[test]
fn count() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default()
        .with_fast_forward(true)
        .with_timeout(Duration::seconds(3));
    let (_, envs) =
        boomerang_util::runner::build_and_test_reactor::<Count<i32>>("count", 0, config).unwrap();
    let count = envs[0]
        .find_reactor_by_name("count")
        .and_then(|r| r.get_state::<i32>())
        .unwrap();
    assert_eq!(*count, 4);
}
