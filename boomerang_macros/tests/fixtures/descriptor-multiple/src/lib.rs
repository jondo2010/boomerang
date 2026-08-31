use boomerang::prelude::*;

#[reactor(
    contract = "example.first",
    contract_version = 1,
    bounds(queue_capacity = 1, payload_bytes = 1, state_bytes = 1, scratch_bytes = 1)
)]
pub fn First() -> impl Reactor {}

#[reactor(
    contract = "example.second",
    contract_version = 1,
    bounds(queue_capacity = 1, payload_bytes = 1, state_bytes = 1, scratch_bytes = 1)
)]
pub fn Second() -> impl Reactor {}
