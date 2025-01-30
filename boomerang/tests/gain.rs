// Example in the Wiki.

use boomerang::prelude::*;

#[reactor_ports]
struct ScalePorts {
    #[input]
    x: u32,
    #[output]
    y: u32,
}

fn scale(scale: u32) -> impl Reactor2<(), Ports = ScalePorts> {
    ScalePorts::build_with(move |builder, (x, y)| {
        builder
            .add_reaction2(None)
            .with_trigger(x)
            .with_effect(y)
            .with_reaction_fn(move |_ctx, _state, (x, mut y)| {
                *y = Some(scale * x.unwrap());
            })
            .finish()?;

        Ok(())
    })
}

#[reactor_ports]
struct TestPorts {
    #[input]
    x: u32,
}

fn test() -> impl Reactor2<(), Ports = TestPorts> {
    TestPorts::build_with(move |builder, (x,)| {
        builder
            .add_reaction2(None)
            .with_trigger(x)
            .with_reaction_fn(move |_ctx, _state, (x,)| {
                println!("Received {:?}", *x);
                assert_eq!(*x, Some(2), "Expected Some(2)!");
            })
            .finish()?;
        Ok(())
    })
}

fn gain() -> impl Reactor2<()> {
    move |name: &str, state, parent, bank_info, is_enclave, env: &mut EnvBuilder| {
        let mut builder = env.add_reactor(name, parent, bank_info, state, is_enclave);
        let g = builder.add_child_reactor2(scale(2), "g", (), false)?;
        let t = builder.add_child_reactor2(test(), "t", (), false)?;
        let tim = builder.add_timer("tim", TimerSpec::STARTUP)?;
        builder.connect_port(g.y, t.x, None, false)?;
        builder
            .add_reaction2(None)
            .with_trigger(tim)
            .with_effect(g.x)
            .with_reaction_fn(move |_ctx, _state, (_tim, mut g_x)| {
                *g_x = Some(1);
            })
            .finish()?;
        Ok(())
    }
}

#[test]
fn test_gain() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default().with_fast_forward(true);
    let _ = boomerang_util::runner::build_and_test_reactor2(gain(), "gain", (), config).unwrap();
}
