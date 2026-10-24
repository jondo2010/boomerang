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
                    #[cfg(not(feature = "runtime-failure"))]
                    {
                        assert_eq!(ctx.get_tag().offset(), Duration::milliseconds(1));
                        println!("sensor received command 42 at logical 1000000 ns");
                    }
                }
            }
        }
    }
}
