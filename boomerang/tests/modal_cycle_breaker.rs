use boomerang::prelude::*;

#[reactor]
fn ModalCycleBreaker(
    #[input] left_in: (),
    #[input] right_in: (),
    #[output] left_out: (),
    #[output] right_out: (),
) -> impl Reactor<Ports = ModalCycleBreakerPorts> {
    mode! { initial left {
        reaction! {
            (left_in) -> left_out {
                *left_out = Some(());
            }
        }
    } }

    mode! { right {
        reaction! {
            (right_in) -> right_out {
                *right_out = Some(());
            }
        }
    } }

    builder.connect_port(left_out, right_in, None, false)?;
    builder.connect_port(right_out, left_in, None, false)?;

    reaction! {
        (startup) {
            ctx.schedule_shutdown(None);
        }
    }
}

#[reactor]
fn ModalCycleChild(
    #[input] input: (),
    #[output] output: (),
) -> impl Reactor<Ports = ModalCycleChildPorts> {
    reaction! {
        (input) -> output {
            *output = Some(());
        }
    }
}

#[reactor]
fn ModalChildCycleBreaker() -> impl Reactor {
    let mut left_child = None;
    let mut right_child = None;

    mode! { initial left {
        left_child = Some(builder.add_child_reactor(ModalCycleChild(), "left_child", (), false)?);
    } }

    mode! { right {
        right_child = Some(builder.add_child_reactor(ModalCycleChild(), "right_child", (), false)?);
    } }

    let left_child = left_child.expect("left child should be built");
    let right_child = right_child.expect("right child should be built");

    builder.connect_port(left_child.output, right_child.input, None, false)?;
    builder.connect_port(right_child.output, left_child.input, None, false)?;

    reaction! {
        (startup) {
            ctx.schedule_shutdown(None);
        }
    }
}

#[test]
fn modal_cycle_breaker_allows_mutually_exclusive_dependencies() {
    let _ = tracing_subscriber::fmt::try_init();
    let config = runtime::Config::default().with_fast_forward(true);
    boomerang_util::runner::build_and_test_reactor(
        ModalCycleBreaker(),
        "modal_cycle_breaker",
        (),
        config,
    )
    .unwrap();
}

#[test]
fn modal_cycle_breaker_allows_child_reactors_in_mutually_exclusive_modes() {
    let _ = tracing_subscriber::fmt::try_init();
    let config = runtime::Config::default().with_fast_forward(true);
    boomerang_util::runner::build_and_test_reactor(
        ModalChildCycleBreaker(),
        "modal_child_cycle_breaker",
        (),
        config,
    )
    .unwrap();
}
