macro_rules! declare_controller {
    ($emit:ident) => {
        $emit! {
            pub mod controller {
                // These helpers belong only to target payload emission.
                fn initial_count() -> u32 { 7 }
                #[reactor(contract = "example.controller", contract_version = 1,
                    bounds(queue_capacity = 16, payload_bytes = 1024,
                           state_bytes = 512, scratch_bytes = 256))]
                pub fn Controller(#[state(default = initial_count())] count: u32) -> impl Reactor {
                    reaction! { tick(startup) {
                        state.count += 1;
                        assert_eq!(state.count, 8);
                        crate::REACTIONS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    } }
                }
            }
        }
    };
}
