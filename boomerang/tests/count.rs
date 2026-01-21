use std::fmt::Debug;

use boomerang::prelude::*;
trait CountData:
    Default + Debug + Copy + runtime::ReactorData + std::ops::AddAssign<i32> + std::cmp::PartialEq<i32>
{
}

impl CountData for i32 {}

#[reactor]
fn Count<T: CountData>(
    #[output] c: T,
    #[state] count: T,
) -> impl Reactor<CountState<T>, Ports = CountPorts<T>> {
    let t = builder.add_timer("t", TimerSpec::default().with_period(Duration::seconds(1)))?;
    let shutdown = builder.get_shutdown_action();

    builder
        .add_reaction(None)
        .with_trigger(t)
        .with_effect(c)
        .with_reaction_fn(|_ctx, state, (_t, mut c)| {
            state.count += 1;
            assert!(c.is_none());
            *c = Some(state.count);
        })
        .finish()?;

    builder
        .add_reaction(None)
        .with_trigger(shutdown)
        .with_reaction_fn(|_ctx, state, _| {
            assert_eq!(state.count, 4, "expected 4, got {:?}", state.count);
            println!("ok");
        })
        .finish()?;
}

#[test]
fn main() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default()
        .with_fast_forward(true)
        .with_timeout(Duration::seconds(3));
    let (_, envs) = boomerang_util::runner::build_and_test_reactor(
        Count::<i32>(),
        "count",
        Default::default(),
        config,
    )
    .unwrap();
    let state = envs[0]
        .find_reactor_by_name("count")
        .and_then(|r| r.get_state::<CountState<i32>>())
        .unwrap();
    assert_eq!(state.count, 4);
}
