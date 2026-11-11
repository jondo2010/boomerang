use boomerang::prelude::*;
use boomerang::runtime::{OutputFault, OutputOutcome};
use std::sync::{Arc, Mutex};

#[reactor(external_effects = [Apply, Stop])]
fn Driver(#[state] calls: Arc<Mutex<Vec<&'static str>>>) -> impl Reactor {
    reaction! { Apply(startup) {
        state.calls.lock().unwrap().push("apply");
        OutputOutcome::Committed
    } }
    reaction! { Stop(shutdown) {
        state.calls.lock().unwrap().push("stop");
        return OutputOutcome::Unchanged;
    } }
}

#[reactor(external_effects = [Apply])]
fn Failing(#[state] calls: Arc<Mutex<Vec<&'static str>>>) -> impl Reactor {
    reaction! { Apply(startup) {
        state.calls.lock().unwrap().push("partial");
        OutputOutcome::Failed(OutputFault::Device)
    } }
    reaction! { Stop(shutdown) { state.calls.lock().unwrap().push("shutdown"); } }
}

#[reactor(external_effects = [Apply])]
fn MissingReport() -> impl Reactor {
    reaction! { Apply(startup) { OutputOutcome::NotReported } }
}

#[reactor]
fn Ordinary(#[state] calls: usize) -> impl Reactor {
    reaction! { Start(startup) { state.calls += 1; } }
}

#[reactor]
fn Pair(left: Arc<Mutex<Vec<&'static str>>>, right: Arc<Mutex<Vec<&'static str>>>) -> impl Reactor {
    ctx.add_child_reactor(
        Driver(),
        "left",
        DriverState {
            calls: left.clone(),
        },
        false,
    )?;
    ctx.add_child_reactor(
        Driver(),
        "right",
        DriverState {
            calls: right.clone(),
        },
        false,
    )?;
}

#[test]
fn multiple_effects_and_instances_keep_their_own_order() {
    let left = Arc::new(Mutex::new(Vec::new()));
    let right = Arc::new(Mutex::new(Vec::new()));
    boomerang_util::runner::build_and_test_reactor(
        Pair(left.clone(), right.clone()),
        "pair",
        (),
        runtime::Config::default(),
    )
    .unwrap();
    assert_eq!(*left.lock().unwrap(), ["apply", "stop"]);
    assert_eq!(*right.lock().unwrap(), ["apply", "stop"]);
    boomerang_util::runner::build_and_test_reactor(
        Ordinary(),
        "ordinary",
        OrdinaryState { calls: 0 },
        runtime::Config::default().with_fast_forward(true),
    )
    .unwrap();
}

#[test]
fn failed_effect_stops_before_shutdown_callbacks() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let result = boomerang_util::runner::build_and_test_reactor(
        Failing(),
        "failing",
        FailingState {
            calls: calls.clone(),
        },
        runtime::Config::default(),
    );
    assert!(result.is_err());
    assert_eq!(*calls.lock().unwrap(), ["partial"]);
}

#[test]
fn declared_effect_cannot_omit_its_report() {
    assert!(boomerang_util::runner::build_and_test_reactor(
        MissingReport(),
        "missing",
        (),
        runtime::Config::default(),
    )
    .is_err());
}

#[test]
fn fast_forward_rejects_before_physical_side_effect() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let config = runtime::Config::default().with_fast_forward(true);
    let mut assembly = Assembly::new();
    Driver()
        .build(
            "early",
            DriverState {
                calls: calls.clone(),
            },
            None,
            None,
            None,
            false,
            &mut assembly,
        )
        .unwrap();
    let runtime = assembly.into_runtime_assembly(&config).unwrap();
    let (key, enclave) = runtime.into_enclaves().next().unwrap();
    let error = runtime::Scheduler::try_new(key, enclave, config, None).unwrap_err();
    assert!(error.to_string().contains("fast-forward"));
    assert!(calls.lock().unwrap().is_empty());
}

#[reactor]
fn Bound(
    #[param] declared: bool,
    #[param] output: OutputOutcome,
    #[state] calls: Arc<Mutex<usize>>,
) -> impl Reactor {
    ctx.add_reaction(Some("bound"))
        .with_external_effect(declared)
        .with_startup_trigger()
        .with_reaction_fn(move |_ctx, state, (_startup,)| {
            *state.calls.lock().unwrap() += 1;
            output
        })
        .finish()?;
}

#[test]
fn live_manual_bindings_enforce_the_same_output_contract() {
    for (declared, output, valid) in [
        (false, OutputOutcome::NotReported, true),
        (false, OutputOutcome::Committed, false),
        (true, OutputOutcome::NotReported, false),
        (true, OutputOutcome::Unchanged, true),
        (true, OutputOutcome::Committed, true),
        (true, OutputOutcome::Failed(OutputFault::Device), false),
    ] {
        let calls = Arc::new(Mutex::new(0));
        let result = boomerang_util::runner::build_and_test_reactor(
            Bound(declared, output),
            "bound",
            BoundState {
                calls: calls.clone(),
            },
            runtime::Config::default(),
        );
        assert_eq!(
            result.is_ok(),
            valid,
            "declared={declared}, output={output:?}"
        );
        assert_eq!(*calls.lock().unwrap(), 1);
    }
}
