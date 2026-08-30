use boomerang::prelude::*;

#[reactor(contract = " example.invalid", contract_version = 1)]
pub fn InvalidContract() -> impl Reactor {}
