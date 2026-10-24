# Ordinary Cargo vehicle federation

Build each application from this directory:

```sh
cargo build --locked -p native-vehicle-controller
cargo build --locked -p native-vehicle-sensor
cargo build --locked -p native-vehicle-rti
cargo test --locked -p native-vehicle-deployment
```

Run the process integration test from the repository root:

```sh
bash boomerang_codegen/tests/run-applications.sh
```

Run `native-vehicle-rti` first. Set `BOOMERANG_RTI_ADDRESS` on both members to its reported address.
Read `native-vehicle-deployment` rustdoc for architecture and configuration inputs.
