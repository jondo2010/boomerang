#[cfg(not(feature = "__boomerang_descriptor"))]
compile_error!("sensor-mcu-payload-only");
#[cfg(feature = "__boomerang_payload")]
compile_error!("sensor-mcu-payload-only");

use boomerang::prelude::*;

#[reactor(contract = "vehicle.sensor", contract_version = 1)]
pub fn Sensor() -> impl Reactor {
    reaction! {
        sample (startup) {
            #[cfg(not(feature = "__boomerang_descriptor"))]
            let _payload_only = "sensor-mcu-payload-only";
        }
    }
}
