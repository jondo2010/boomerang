//! Check multiport capabilities on Outputs.
//!
//! Ported from LF https://github.com/lf-lang/lingua-franca/blob/master/test/C/src/concurrent/ThreadedMultiport.lf

use boomerang::prelude::*;

#[derive(Debug, Default, Clone)]
pub struct State {
    s: i32,
}

#[reactor(state = State)]
fn Source<const WIDTH: usize>(#[output] out: [i32; WIDTH]) -> impl Reactor {
    timer! { t(200 msec) };

    builder
        .add_reaction(None)
        .with_trigger(t)
        .with_effect(out)
        .with_reaction_fn(|_ctx, state, (_t, mut out)| {
            for o in out.iter_mut() {
                **o = Some(state.s);
            }
            state.s += 1;
        })
        .finish()?;
}

#[reactor]
fn Computation<const ITERS: usize>(#[input] in_: i32, #[output] out: i32) -> impl Reactor {
    builder
        .add_reaction(None)
        .with_trigger(in_)
        .with_effect(out)
        .with_reaction_fn(move |_ctx, _state, (in_, mut out)| {
            let mut offset = 0;
            //std::thread::sleep(std::time::Duration::nanosecondss(1));
            for _ in 0..ITERS {
                offset += 1;
            }
            *out = in_.map(|x| x + offset);
        })
        .finish()?;
}

#[reactor(state = State)]
fn Destination<const WIDTH: usize, const ITERS: usize = 100_000_000>(
    #[input] in_: [i32; WIDTH],
) -> impl Reactor {
    builder
        .add_reaction(None)
        .with_trigger(in_)
        .with_reaction_fn(move |_ctx, state, (in_,)| {
            let expected = ITERS as i32 * WIDTH as i32 + state.s;
            let sum = in_.iter().filter_map(|x| x.as_ref()).sum::<i32>();
            println!("Sum of received: {}.", sum);
            assert_eq!(sum, expected, "Expected {expected}.");
            state.s += WIDTH as i32;
        })
        .finish()?;

    builder
        .add_reaction(None)
        .with_shutdown_trigger()
        .with_reaction_fn(move |_ctx, state, (_shutdown,)| {
            assert!(state.s > 0, "ERROR: Destination received no input!");
            println!("Success.");
        })
        .finish()?;
}

#[reactor]
fn ThreadingMultiport<const WIDTH: usize = 4, const ITERS: usize = 100_000_000>() -> impl Reactor {
    let a = builder.add_child_reactor(Source::<WIDTH>(), "a", Default::default(), false)?;
    let t: [_; WIDTH] =
        builder.add_child_reactors(Computation::<ITERS>(), "t", Default::default(), false)?;
    let b = builder.add_child_reactor(
        Destination::<WIDTH, ITERS>(),
        "b",
        Default::default(),
        false,
    )?;

    builder.connect_ports(a.out.iter().copied(), t.iter().map(|c| c.in_), None, false)?;
    builder.connect_ports(
        t.iter().flat_map(|c| c.out.iter()).copied(),
        b.in_.iter().copied(),
        None,
        false,
    )?;
}

#[test]
fn threading_multiport() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default()
        .with_fast_forward(true)
        .with_timeout(Duration::seconds(2));
    let _ = boomerang_util::runner::build_and_test_reactor(
        ThreadingMultiport::<4, 100_000>(),
        "threading_multiport",
        (),
        config,
    )
    .unwrap();
}
