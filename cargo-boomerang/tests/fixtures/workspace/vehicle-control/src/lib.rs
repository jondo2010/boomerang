#[cfg(not(feature = "__boomerang_descriptor"))]
compile_error!("controller-payload-only");
#[cfg(feature = "__boomerang_payload")]
compile_error!("controller-payload-only");

use boomerang::prelude::*;

#[reactor(
    contract = "vehicle.controller",
    contract_version = 1,
    bounds(
        queue_capacity = 16,
        payload_bytes = 1024,
        state_bytes = 512,
        scratch_bytes = 256,
    )
)]
pub fn Controller() -> impl Reactor {
    reaction! {
        control (startup) {
            #[cfg(not(feature = "__boomerang_descriptor"))]
            let _payload_only = "controller-payload-only";
        }
    }
    mode! { initial active {
        reaction! { (shutdown) {} }
    } }
}
