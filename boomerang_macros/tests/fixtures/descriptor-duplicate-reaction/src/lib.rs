use boomerang::prelude::*;

#[reactor(contract = "example.duplicate", contract_version = 1)]
pub fn Duplicate() -> impl Reactor {
    reaction! { same (startup) {} }
    reaction! { same (shutdown) {} }
}
