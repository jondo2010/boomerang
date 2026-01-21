// Example in the Wiki.

use boomerang::prelude::*;

#[reactor]
fn Scale(#[input] x: u32, #[output] y: u32, scale: u32) -> impl Reactor {
    reaction! {
        (x) -> y {
            // Scale the input value by the specified scale factor
            *y = Some(scale * x.unwrap());
        }
    }
}

#[reactor]
/// This reactor simply receives a value and prints it
fn Test(#[input] x: u32) -> impl Reactor {
    reaction! {
        (x) {
            println!("Received value: {:?}", *x);
            assert_eq!(*x, Some(2), "Expected value to be 2!");
        }
    }
}

#[reactor]
fn Gain() -> impl Reactor {
    let g = builder.add_child_reactor(Scale(2), "g", (), false)?;
    let t = builder.add_child_reactor(Test(), "t", (), false)?;
    let tim = builder.add_timer("tim", TimerSpec::STARTUP)?;
    builder.connect_port(g.y, t.x, None, false)?;

    reaction! {
        (tim) -> g.x {
            *g_x = Some(1);
        }
    }
}

#[test]
fn test_gain() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default().with_fast_forward(true);
    let _ = boomerang_util::runner::build_and_test_reactor(Gain(), "gain", (), config).unwrap();
}
