boomerang::component! {
    pub mod oracle {
        use boomerang::prelude::*;
        fn initial_count() -> usize { 3 }
        fn target_only_reaction_payload() {}
#[reactor(
    contract = "example.sensor",
    contract_version = 1,
    bounds(
        queue_capacity = 16,
        payload_bytes = 1024,
        state_bytes = 512,
        scratch_bytes = 256,
    )
)]
pub fn r#match(
    #[input] r#async: u32,
    #[output] r#type: u32,
    #[output] r#yield: u32,
    #[state(default = initial_count())] r#loop: usize,
) -> impl Reactor {
    reaction! {
        r#move (r#async, startup) -> r#type, r#yield {
            target_only_reaction_payload();
            state.r#loop += r#async.unwrap_or_default() as usize;
            r#type.set(ctx);
            *r#yield = Some(state.r#loop as u32);
        }
    }
    mode! { r#type {
        reaction! {
            (reset) -> history(r#type) {
                target_only_reaction_payload();
                r#type.set(ctx);
            }
        }
    } }
}
    }
}
#[test]
fn raw_identifiers_modes_and_anonymous_reactions_preserve_exported_names() {
    #[cfg(feature = "metadata")]
    {
        let descriptor = oracle::__boomerang::metadata::descriptor();
        assert_eq!(descriptor.reaction_slots()[1].id.to_string(), "Match/#g0");
        assert_eq!(descriptor.state_slots()[0].id.to_string(), "Match/loop");
    }
    #[cfg(feature = "payload")]
    {
        assert_eq!(oracle::__boomerang::payload::state_Match().r#loop, 3);
    }
}
