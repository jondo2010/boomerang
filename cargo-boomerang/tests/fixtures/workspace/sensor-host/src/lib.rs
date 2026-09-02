#[cfg(not(any(feature = "__boomerang_descriptor", feature = "__boomerang_payload")))]
compile_error!("sensor-mcu-payload-only");

use boomerang::prelude::*;

#[reactor(
    contract = "vehicle.sensor",
    contract_version = 1,
    bounds(
        queue_capacity = 8,
        payload_bytes = 512,
        state_bytes = 256,
        scratch_bytes = 128,
    )
)]
pub fn Sensor(#[input] command: u32) -> impl Reactor {
    reaction! {
        sample (command) {
            #[cfg(not(feature = "__boomerang_descriptor"))]
            let _payload_only = "sensor-mcu-payload-only";
            assert_eq!(*command, Some(42));
            ctx.schedule_shutdown(None);
        }
    }
}
