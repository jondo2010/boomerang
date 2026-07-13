use boomerang::prelude::*;

#[reactor]
fn ModalStructuralSyntax(#[state] a_count: u32, #[state] b_count: u32) -> impl Reactor {
    let pulse = ctx.add_logical_action::<()>("pulse", None)?;

    reaction! {
        (startup) -> pulse {
            ctx.schedule_action(&mut pulse, (), Some(Duration::nanoseconds(1)));
            ctx.schedule_action(&mut pulse, (), Some(Duration::nanoseconds(2)));
        }
    }

    mode! { initial mode_a {
        reaction! {
            (pulse) -> mode_b {
                state.a_count += 1;
                mode_b.set(ctx);
            }
        }
    } }

    mode! { mode_b {
        reaction! {
            (pulse) -> mode_a {
                state.b_count += 1;
                mode_a.set(ctx);
                if state.b_count == 1 {
                    ctx.schedule_shutdown(None);
                }
            }
        }
    } }

    reaction! {
        (shutdown) {
            assert_eq!(state.a_count, 1, "mode_a reaction should run once");
            assert_eq!(state.b_count, 1, "mode_b reaction should run once");
        }
    }
}

#[test]
fn modal_structural_syntax() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default().with_fast_forward(true);
    boomerang_util::runner::build_and_test_reactor(
        ModalStructuralSyntax(),
        "modal_structural_syntax",
        ModalStructuralSyntaxState {
            a_count: 0,
            b_count: 0,
        },
        config,
    )
    .unwrap();
}
