use boomerang::prelude::*;

#[reactor]
fn ModalBankNode<const NUM_NODES: usize>(
    #[state] received: bool,
    #[input] input: [usize; NUM_NODES],
    #[output] output: usize,
) -> impl Reactor {
    reaction! {
        (startup) -> output {
            *output = ctx.get_bank_index();
        }
    }

    reaction! {
        (input) {
            let received = input.iter().filter_map(|port| **port).collect::<Vec<_>>();
            assert_eq!(
                received.len(),
                NUM_NODES,
                "bank node should receive one value from every peer"
            );
            for expected in 0..NUM_NODES {
                assert!(
                    received.contains(&expected),
                    "bank node should receive value {expected}"
                );
            }
            state.received = true;
        }
    }

    reaction! {
        (shutdown) {
            assert!(state.received, "bank node received no modal input");
        }
    }
}

#[reactor]
fn ModalMultiportBank<const NUM_NODES: usize = 4>() -> impl Reactor {
    mode! { initial active {
        let nodes: [_; NUM_NODES] = builder.add_child_reactors(
            ModalBankNode::<NUM_NODES>(),
            "nodes",
            Default::default(),
            false,
        )?;

        builder.connect_ports(
            nodes
                .iter()
                .flat_map(|child| child.output.iter())
                .copied()
                .cycle()
                .take(NUM_NODES * NUM_NODES),
            nodes.iter().flat_map(|child| child.input.iter()).copied(),
            None,
            false,
        )?;
    } }

    mode! { idle {
    } }

    reaction! {
        (startup) {
            ctx.schedule_shutdown(Some(Duration::nanoseconds(1)));
        }
    }
}

#[test]
fn modal_multiport_bank_routes_inside_active_mode() {
    let _ = tracing_subscriber::fmt::try_init();
    let config = runtime::Config::default().with_fast_forward(true);
    boomerang_util::runner::build_and_test_reactor(
        ModalMultiportBank::<4>(),
        "modal_multiport_bank",
        (),
        config,
    )
    .unwrap();
}
