use boomerang::prelude::*;
use boomerang_util::timeout;

use std::time::Duration;

trait CountData:
    Copy + runtime::ReactorData + std::ops::AddAssign<i32> + std::cmp::PartialEq<i32>
{
}

impl CountData for i32 {}

#[derive(Reactor)]
#[reactor(state = "T", reaction = "ReactionT<T>", reaction = "ReactionShutdown")]
struct Count<T: CountData> {
    #[reactor(timer(period = "1 msec"))]
    t: TimerActionKey,
    c: TypedPortKey<T, Output>,
    #[reactor(child = "Duration::from_secs(1)")]
    _timeout: timeout::Timeout,
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
        assert_eq!(*state, 1e3 as i32, "expected 1e3, got {state:?}");
        println!("ok");
    }
}

#[test]
fn count() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default().with_fast_forward(true);
    let (_, sched) =
        boomerang_util::runner::build_and_test_reactor::<Count<i32>>("count", 0, config).unwrap();
    let env = sched.into_env();
    let count = env
        .find_reactor_by_name("count")
        .and_then(|r| r.get_state::<i32>())
        .unwrap();
    assert_eq!(*count, 1e3 as i32);
}
