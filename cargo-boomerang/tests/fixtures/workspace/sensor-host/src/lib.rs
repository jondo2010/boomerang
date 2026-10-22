#[cfg(boomerang_facet = "payload")]
const _: () = assert!(
    option_env!("BOOMERANG_PAYLOAD_INPUT_V1_MACRO_ABI").is_some(),
    "sensor-mcu-payload-only requires payload compile inputs"
);

boomerang::component! {
    pub mod sensor {
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
                    assert_eq!(*command, Some(42));
                    #[cfg(feature = "runtime-failure")]
                    std::process::exit(42);
                    #[cfg(feature = "timing-check")]
                    {
                        assert_eq!(ctx.get_tag().offset(), Duration::milliseconds(1));
                        println!("sensor received command 42 at logical 1000000 ns");
                    }
                    #[cfg(not(feature = "timing-check"))]
                    println!("sensor received command 42");
                    #[cfg(not(feature = "natural-quiescence"))]
                    {
                        eprintln!("sensor scheduling shutdown");
                        ctx.schedule_shutdown(None);
                    }
                }
            }
        }
    }
}
