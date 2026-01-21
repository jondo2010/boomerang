//! This tests generic port data as well as a delayed connection

use boomerang::prelude::*;

mod delay_int;

#[reactor]
fn Delay<T: runtime::ReactorData + Clone>(
    #[output] out: T,
    #[input] in_: T,
    delay: Duration,
) -> impl Reactor {
    let a = builder.add_logical_action("a", Some(delay))?;

    let _ = builder
        .add_reaction(None)
        .with_trigger(a)
        .with_effect(out)
        .with_reaction_fn(|ctx, _state, (mut a, mut out)| {
            *out = ctx.get_action_value(&mut a).cloned();
        })
        .finish()?;

    let _ = builder
        .add_reaction(None)
        .with_trigger(in_)
        .with_effect(a)
        .with_reaction_fn(|ctx, _state, (in_, mut a)| {
            ctx.schedule_action(&mut a, in_.clone().unwrap(), None);
        })
        .finish()?;
}

#[reactor]
fn GenericAfter() -> impl Reactor {
    let d =
        builder.add_child_reactor(Delay::<u32>(Duration::milliseconds(50)), "delay", (), false)?;

    let test = builder.add_child_reactor(
        delay_int::Test(),
        "test",
        delay_int::TestState {
            start_time: std::time::Instant::now(),
        },
        false,
    )?;

    builder.connect_port(d.out, test.in_, Some(Duration::milliseconds(50)), false)?;

    builder
        .add_reaction(None)
        .with_startup_trigger()
        .with_effect(d.in_)
        .with_reaction_fn(move |_ctx, _state, (_, mut d_in)| {
            *d_in = Some(42);
        })
        .finish()?;
}

#[test]
fn generic_after() {
    let config = runtime::Config::default()
        .with_fast_forward(true)
        .with_timeout(Duration::seconds(1));
    let (_, _env) = boomerang_util::runner::build_and_test_reactor(
        GenericAfter(),
        "generic_after",
        (),
        config,
    )
    .unwrap();
}
