use boomerang::prelude::*;

#[reactor(
    contract = "example.overflow",
    contract_version = 18446744073709551616
)]
pub fn Overflow() -> impl Reactor {}
