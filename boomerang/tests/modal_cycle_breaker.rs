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
