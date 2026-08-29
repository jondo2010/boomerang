use boomerang::prelude::*;

#[allow(dead_code)]
fn init_state() {}

/// Compile-fail fixture for an initializer without a custom state type.
#[reactor(state_init = init_state)]
fn InvalidStateInit() -> impl Reactor {}
