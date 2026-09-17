#[cfg(boomerang_facet = "payload")]
const _: () = assert!(
    option_env!("BOOMERANG_PAYLOAD_INPUT_V1_MACRO_ABI").is_some(),
    "controller-payload-only requires payload compile inputs"
);

#[cfg(all(feature = "broken-payload", boomerang_facet = "payload"))]
compile_error!("intentional target payload build failure");
#[cfg(all(feature = "broken-descriptor", boomerang_facet = "descriptor"))]
compile_error!("intentional descriptor build failure");
#[cfg(all(
    feature = "profile-config-probe",
    boomerang_facet = "payload",
    debug_assertions
))]
compile_error!("profile-config probe requires the release profile");
#[cfg(all(
    feature = "profile-config-probe",
    boomerang_facet = "payload",
    not(boomerang_cargo_config_probe)
))]
compile_error!("profile-config probe requires Cargo configuration rustflags");

/// Rejects compiling the controller payload under the sensor Federate's Cargo configuration.
#[cfg(boomerang_facet = "payload")]
const _: () = assert!(
    option_env!("BOOMERANG_SENSOR_SLICE_SENTINEL").is_none(),
    "sensor slice compiled the vehicle-control payload"
);

boomerang::component! {
    pub mod controller {
        use boomerang::prelude::*;

        #[cfg(all(feature = "warning-diagnostic", boomerang_facet = "payload"))]
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
                    *command = Some(42);
                }
            }
            mode! { initial active {
                reaction! { (shutdown) {} }
            } }
        }
    }
}
