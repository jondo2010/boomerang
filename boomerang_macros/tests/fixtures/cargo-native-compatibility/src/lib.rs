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
const EXPECTED: boomerang::runtime::binding::DescriptorFingerprint =
    boomerang::runtime::binding::DescriptorFingerprint::new([
        0xbd, 0x8f, 0x55, 0x17, 0xc7, 0xf1, 0x42, 0x86, 0x3a, 0xe7, 0xca, 0xb5, 0xf9, 0x0e, 0x52,
        0x25, 0xf8, 0x03, 0xfd, 0x74, 0xe4, 0xf8, 0x7a, 0x45, 0xf6, 0xaf, 0x53, 0x95, 0xd9, 0xe0,
        0x5a, 0x60,
    ]);
#[cfg(feature = "payload")]
const _: () = boomerang::runtime::binding::assert_descriptor_fingerprint(
    EXPECTED,
    oracle::__boomerang::payload::binding_manifest().descriptor_fingerprint(),
);
#[test]
fn raw_identifiers_modes_and_anonymous_reactions_preserve_the_encoding_oracle() {
    #[cfg(feature = "metadata")]
    {
        let descriptor = oracle::__boomerang::metadata::descriptor();
        assert_eq!(descriptor.fingerprint(), EXPECTED);
        assert_eq!(descriptor.reaction_slots()[1].id.to_string(), "Match/#g0");
        assert_eq!(descriptor.state_slots()[0].id.to_string(), "Match/loop");
    }
    #[cfg(feature = "payload")]
    {
        assert_eq!(oracle::__boomerang::payload::state_Match().r#loop, 3);
        assert_eq!(
            oracle::__boomerang::payload::binding_manifest().descriptor_fingerprint(),
            EXPECTED
        );
    }
}
