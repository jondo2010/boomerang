//! Dependency-exclusion sentinel for generated sensor-slice launchers.

/// Rejects compiling the controller payload under the sensor Federate's Cargo configuration.
fn main() {
    println!("cargo:rerun-if-env-changed=BOOMERANG_SENSOR_SLICE_SENTINEL");
    if std::env::var_os("BOOMERANG_SENSOR_SLICE_SENTINEL").is_some()
        && std::env::var_os("CARGO_FEATURE___BOOMERANG_PAYLOAD").is_some()
    {
        panic!("sensor slice compiled the vehicle-control payload");
    }
}
