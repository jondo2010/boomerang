use boomerang::prelude::*;

#[reactor]
fn InactiveChild() -> impl Reactor {
    reaction! {
        (startup) {
            panic!("startup reaction in inactive child mode should not run");
        }
    }
}

#[reactor]
fn ModalNestedReactions() -> impl Reactor {
    mode! { initial idle {
    } }

    mode! { active {
        let _inactive_child = ctx.add_child_reactor(InactiveChild(), "inactive_child", (), false)?;
    } }

    reaction! {
        (startup) {
            ctx.schedule_shutdown(None);
        }
    }
}

#[test]
fn modal_nested_reactions() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default().with_fast_forward(true);
    boomerang_util::runner::build_and_test_reactor(
        ModalNestedReactions(),
        "modal_nested_reactions",
        (),
        config,
    )
    .unwrap();
}
