use boomerang::prelude::*;

#[cfg(not(feature = "duplicate-mode"))]
#[reactor(contract = "example.duplicate", contract_version = 1)]
pub fn Duplicate() -> impl Reactor {
    reaction! { same (startup) {} }
    reaction! { same (shutdown) {} }
}

#[cfg(feature = "duplicate-mode")]
#[reactor(contract = "example.duplicate-mode", contract_version = 1)]
pub fn DuplicateMode() -> impl Reactor {
    mode! { initial duplicate {} }
    mode! { duplicate {} }
}
