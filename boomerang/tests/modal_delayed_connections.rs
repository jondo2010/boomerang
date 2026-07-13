use boomerang::prelude::*;

#[reactor]
fn DelayedSource(#[output] out: u32) -> impl Reactor<Ports = DelayedSourcePorts> {
    reaction! {
        (startup) -> out {
            *out = Some(1);
        }
    }
}

#[reactor]
fn DelayedSink(
    #[state] received: bool,
    #[state] received_time_ns: i128,
    #[input] input: u32,
) -> impl Reactor<DelayedSinkState, Ports = DelayedSinkPorts> {
    reaction! {
        (input) {
            assert_eq!(input.as_ref().copied(), Some(1));
            state.received = true;
            state.received_time_ns = ctx.get_elapsed_logical_time().whole_nanoseconds();
            ctx.schedule_shutdown(None);
        }
    }
}

#[reactor]
fn ModalDelayedConnectionHistory() -> impl Reactor {
    let enter = ctx.add_logical_action::<()>("enter", None)?;
    let pause = ctx.add_logical_action::<()>("pause", None)?;
    let resume = ctx.add_logical_action::<()>("resume", None)?;

    reaction! {
        (startup) -> enter, pause, resume {
            ctx.schedule_action(&mut enter, (), Some(Duration::nanoseconds(1)));
            ctx.schedule_action(&mut pause, (), Some(Duration::nanoseconds(3)));
            ctx.schedule_action(&mut resume, (), Some(Duration::nanoseconds(10)));
        }
    }

    mode! { initial idle {
        reaction! {
            (enter) -> active {
                active.set(ctx);
            }
        }

        reaction! {
            (resume) -> history(active) {
                active.set(ctx);
            }
        }
    } }

    mode! { active {
        let source = ctx.add_child_reactor(DelayedSource(), "source", (), false)?;
        let sink = ctx.add_child_reactor(
            DelayedSink(),
            "sink",
            DelayedSinkState {
                received: false,
                received_time_ns: -1,
            },
            false,
        )?;
        ctx.connect_port(
            source.out,
            sink.input,
            Some(Duration::nanoseconds(5)),
            false,
        )?;

        reaction! {
            (pause) -> idle {
                idle.set(ctx);
            }
        }
    } }
}

#[reactor]
fn ModalDelayedConnectionReset() -> impl Reactor {
    let enter = ctx.add_logical_action::<()>("enter", None)?;
    let pause = ctx.add_logical_action::<()>("pause", None)?;
    let resume = ctx.add_logical_action::<()>("resume", None)?;

    reaction! {
        (startup) -> enter, pause, resume {
            ctx.schedule_action(&mut enter, (), Some(Duration::nanoseconds(1)));
            ctx.schedule_action(&mut pause, (), Some(Duration::nanoseconds(3)));
            ctx.schedule_action(&mut resume, (), Some(Duration::nanoseconds(10)));
            ctx.schedule_shutdown(Some(Duration::nanoseconds(15)));
        }
    }

    mode! { initial idle {
        reaction! {
            (enter) -> active {
                active.set(ctx);
            }
        }

        reaction! {
            (resume) -> active {
                active.set(ctx);
            }
        }
    } }

    mode! { active {
        let source = ctx.add_child_reactor(DelayedSource(), "source", (), false)?;
        let sink = ctx.add_child_reactor(
            DelayedSink(),
            "sink",
            DelayedSinkState {
                received: false,
                received_time_ns: -1,
            },
            false,
        )?;
        ctx.connect_port(
            source.out,
            sink.input,
            Some(Duration::nanoseconds(5)),
            false,
        )?;

        reaction! {
            (pause) -> idle {
                idle.set(ctx);
            }
        }
    } }
}

#[test]
fn modal_delayed_connection_history_resumes_pending_delivery() {
    let _ = tracing_subscriber::fmt::try_init();
    let config = runtime::Config::default().with_fast_forward(true);
    let (_, envs) = boomerang_util::runner::build_and_test_reactor(
        ModalDelayedConnectionHistory(),
        "modal_delayed_connection_history",
        (),
        config,
    )
    .unwrap();

    let sink = envs[0]
        .find_reactor_by_name("modal_delayed_connection_history/sink")
        .and_then(|reactor| reactor.get_state::<DelayedSinkState>())
        .expect("sink state");

    assert!(sink.received, "history should resume the pending delivery");
    assert_eq!(
        sink.received_time_ns, 13,
        "delayed connection should fire after the remaining active local delay"
    );
}

#[test]
fn modal_delayed_connection_reset_discards_pending_delivery() {
    let _ = tracing_subscriber::fmt::try_init();
    let config = runtime::Config::default().with_fast_forward(true);
    let (_, envs) = boomerang_util::runner::build_and_test_reactor(
        ModalDelayedConnectionReset(),
        "modal_delayed_connection_reset",
        (),
        config,
    )
    .unwrap();

    let sink = envs[0]
        .find_reactor_by_name("modal_delayed_connection_reset/sink")
        .and_then(|reactor| reactor.get_state::<DelayedSinkState>())
        .expect("sink state");

    assert!(
        !sink.received,
        "reset should discard the pending delayed delivery"
    );
}
