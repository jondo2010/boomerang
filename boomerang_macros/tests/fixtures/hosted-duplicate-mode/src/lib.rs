use boomerang::prelude::*;

#[reactor(contract = "example.hosted", contract_version = 1)]
pub fn hosted_duplicate_mode() -> impl Reactor {
    mode! { initial duplicate {} }
    mode! { duplicate {} }
}
