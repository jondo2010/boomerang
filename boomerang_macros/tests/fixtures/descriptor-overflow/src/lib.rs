use boomerang::prelude::*;

#[reactor(
    contract = "example.overflow",
    contract_version = 18446744073709551616,
    bounds(queue_capacity = 1, payload_bytes = 1, state_bytes = 1, scratch_bytes = 1)
)]
pub fn Overflow() -> impl Reactor {}
