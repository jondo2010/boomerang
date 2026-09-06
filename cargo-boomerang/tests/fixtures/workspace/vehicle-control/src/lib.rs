#[cfg(not(any(feature = "__boomerang_descriptor", feature = "__boomerang_payload")))]
compile_error!("controller-payload-only");
#[cfg(all(feature = "broken-payload", feature = "__boomerang_payload"))]
compile_error!("intentional target payload build failure");
#[cfg(all(
    feature = "broken-descriptor",
    feature = "__boomerang_descriptor"
))]
compile_error!("intentional descriptor build failure");
#[cfg(all(
    feature = "profile-config-probe",
    feature = "__boomerang_payload",
    debug_assertions
))]
compile_error!("profile-config probe requires the release profile");
#[cfg(all(
    feature = "profile-config-probe",
    feature = "__boomerang_payload",
    not(boomerang_cargo_config_probe)
))]
compile_error!("profile-config probe requires Cargo configuration rustflags");

use boomerang::prelude::*;

#[cfg(all(feature = "warning-diagnostic", feature = "__boomerang_payload"))]
const INTENTIONAL_TARGET_PAYLOAD_WARNING: () = ();

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
pub fn Controller(#[output] command: u32) -> impl Reactor {
    reaction! {
        control (startup) -> command {
            #[cfg(not(feature = "__boomerang_descriptor"))]
            let _payload_only = "controller-payload-only";
            *command = Some(42);
        }
    }
    mode! { initial active {
        reaction! { (shutdown) {} }
    } }
}
