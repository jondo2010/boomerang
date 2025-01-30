//! This tests actions with payloads by delaying an input by a fixed amount.
//!
//! Ported from <https://github.com/lf-lang/lingua-franca/blob/master/test/Cpp/src/DelayInt.lf>

use boomerang::prelude::*;

#[reactor_ports]
struct DelayPorts {
    #[input]
    in_: u32,
    #[output]
    out: u32,
}

fn delay(delay: Duration) -> impl Reactor2<(), Ports = DelayPorts> {
    DelayPorts::build_with(move |builder, (in_, out)| {
        let d = builder.add_logical_action::<u32>("d", None)?;
        builder
            .add_reaction2(None)
            .with_trigger(in_)
            .with_effect(d)
            .with_reaction_fn(move |ctx, _state, (in_, mut d)| {
                ctx.schedule_action(&mut d, in_.unwrap(), Some(delay));
            })
            .finish()?;

        builder
            .add_reaction2(None)
            .with_trigger(d)
            .with_effect(out)
            .with_reaction_fn(|ctx, _state, (mut d, mut out)| {
                *out = ctx.get_action_value(&mut d).cloned();
            })
            .finish()?;

        Ok(())
    })
}

#[reactor_ports]
struct TestPorts {
    #[input]
    in_: u32,
}

struct TestState {
    start_time: std::time::Instant,
}

fn test() -> impl Reactor2<TestState, Ports = TestPorts> {
    TestPorts::build_with::<_, TestState>(|builder, (in_,)| {
        let start = builder.get_shutdown_action();

        builder
            .add_reaction2(None)
            .with_trigger(start)
            .with_reaction_fn(move |ctx, state, _| state.start_time = ctx.get_logical_time())
            .finish()?;

        builder
            .add_reaction2(None)
            .with_trigger(in_)
            .with_reaction_fn(|ctx, state, (mut in_,)| {
                println!("Received: {}", in_.unwrap());
                // Check the time of the input.
                let current_time = ctx.get_logical_time();
                let elapsed = current_time - state.start_time;
                println!("After {elapsed:?} of logical time.");
                assert_eq!(
                    elapsed,
                    Duration::milliseconds(100),
                    "Expected 100ms delay."
                );
                assert_eq!(*in_, Some(42), "Expected input to be 42.");
            })
            .finish()?;

        Ok(())
    })
}

#[reactor_ports]
struct DelayIntPorts {}

fn delay_int() -> impl Reactor2<(), Ports = DelayIntPorts> {
    DelayIntPorts::build_with(|builder, _| {
        let t = builder.get_startup_action();
        let d = builder.add_child_reactor2(delay(Duration::milliseconds(100)), "d", (), false)?;
        let test = builder.add_child_reactor2(
            test(),
            "test",
            TestState {
                start_time: std::time::Instant::now(),
            },
            false,
        )?;
        builder.connect_port(d.out, test.in_, None, false)?;
        builder
            .add_reaction2(None)
            .with_trigger(t)
            .with_effect(d.in_)
            .with_reaction_fn(|ctx, state, (t, mut d_in)| {
                *d_in = Some(42);
            })
            .finish()?;
        Ok(())
    })
}
