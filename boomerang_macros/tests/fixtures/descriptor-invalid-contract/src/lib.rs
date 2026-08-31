use boomerang::prelude::*;

#[reactor(
    contract = " example.invalid",
    contract_version = 1,
    bounds(queue_capacity = 1, payload_bytes = 1, state_bytes = 1, scratch_bytes = 1)
)]
pub fn InvalidContract() -> impl Reactor {}
