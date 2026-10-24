boomerang::component! {
    pub mod controller {
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
