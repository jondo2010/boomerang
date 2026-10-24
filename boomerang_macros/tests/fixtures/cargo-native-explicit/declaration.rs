macro_rules! controller_declaration {
    ($emit:ident) => {
        $emit! {
            pub mod controller {
                use boomerang::prelude::*;
                #[cfg(not(feature = "wasm-helper"))]
                use cargo_native_helper::value;
                #[cfg(feature = "wasm-helper")]
                use cargo_native_wasm_helper::value;
                pub fn helper() -> u32 { value() }
                #[reactor(contract = "example.controller", contract_version = 1,
                    bounds(queue_capacity = 16, payload_bytes = 1024,
                           state_bytes = 512, scratch_bytes = 256))]
                pub fn Controller(#[output] command: u32,
                    #[state(default = helper())] count: u32) -> impl Reactor {
                    reaction! { control(startup) -> command {
                        state.count += helper();
                        *command = Some(state.count);
                    } }
                }
            }
        }
    };
}
