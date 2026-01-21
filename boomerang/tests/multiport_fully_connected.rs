use boomerang::prelude::*;

#[reactor]
fn Node<const NUM_NODES: usize>(
    #[state] received: bool,
    #[input] inp: [i32; NUM_NODES],
    #[output] out: i32,
) -> impl Reactor {
    builder
        .add_reaction(Some("Startup"))
        .with_startup_trigger()
        .with_effect(out)
        .with_reaction_fn(|ctx, _state, (_startup, mut out)| {
            println!("Hello from node {}!", ctx.get_bank_index().unwrap());
            // broadcast my ID to everyone
            *out = ctx.get_bank_index().map(|x| x as i32);
        })
        .finish()?;

    builder
        .add_reaction(Some("In"))
        .with_trigger(inp)
        .with_reaction_fn(|ctx, state, (inp,)| {
            let mut count = 0;
            let mut vals = vec![];
            for p in &inp {
                if let Some(val) = **p {
                    state.received = true;
                    count += 1;
                    vals.push(val.to_string());
                }
            }

            println!(
                "Node {} received messages from {}.",
                ctx.get_bank_index().unwrap(),
                vals.join(" "),
            );

            assert_eq!(
                Some(count),
                ctx.get_bank_total(),
                "Received fewer messages than expected!"
            );
        })
        .finish()?;

    builder
        .add_reaction(Some("Shutdown"))
        .with_shutdown_trigger()
        .with_reaction_fn(|_ctx, state, (_shutdown,)| {
            assert!(state.received, "Received no input!");
        })
        .finish()?;
}

#[reactor]
fn Main<const NUM_NODES: usize = 4>() -> impl Reactor {
    let nodes: [_; NUM_NODES] =
        builder.add_child_reactors(Node::<4>(), "nodes", Default::default(), false)?;
    builder.connect_ports(
        nodes
            .iter()
            .flat_map(|child| child.out.iter())
            .copied()
            .cycle(),
        nodes.iter().flat_map(|child| child.inp.iter()).copied(),
        None,
        false,
    )?;
}

#[test]
fn mutliport_fully_connected() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default().with_fast_forward(true);
    let _ = boomerang_util::runner::build_and_test_reactor(
        Main::<4>(),
        "multiport_fully_connected",
        (),
        config,
    )
    .unwrap();
}
