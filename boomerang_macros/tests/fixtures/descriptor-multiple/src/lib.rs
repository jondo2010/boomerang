use boomerang::prelude::*;

#[reactor(contract = "example.first", contract_version = 1)]
pub fn First() -> impl Reactor {}

#[reactor(contract = "example.second", contract_version = 1)]
pub fn Second() -> impl Reactor {}
