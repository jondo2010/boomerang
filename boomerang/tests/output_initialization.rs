//! Output observer initialization requires compiled execution.
#![cfg(feature = "hosted-output")]
use boomerang::builder;
use boomerang::prelude::*;
use boomerang::runtime::OutputOutcome;
use std::sync::atomic::{AtomicUsize, Ordering};

static INITIALIZER_CALLS: AtomicUsize = AtomicUsize::new(0);
#[derive(Debug)]
struct DriverState;
impl DriverState {
    // The live facet must reject this capability without referencing the constructor.
    #[allow(dead_code)]
    fn new(_init: DriverInit) -> Result<Self, std::io::Error> {
        INITIALIZER_CALLS.fetch_add(1, Ordering::SeqCst);
        Ok(Self)
    }
}
#[reactor(state = DriverState, state_init = DriverState::new, init_context(name = DriverInit, output_observer = true), external_effects = [Apply])]
fn Driver() -> impl Reactor {
    reaction! { Apply(startup) { OutputOutcome::Committed } }
}

#[test]
fn live_output_observer_rejects_before_user_initialization() {
    let mut assembly = builder::Assembly::new();
    let result = Driver().build_initialized(
        "driver",
        None,
        None,
        None,
        builder::ReactorPlacement::Local,
        &mut assembly,
    );
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("live output observers must require compiled execution"),
    };
    assert!(error.to_string().contains("compiled"), "{error}");
    assert_eq!(INITIALIZER_CALLS.load(Ordering::SeqCst), 0);
}
