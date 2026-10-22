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
        0xfb, 0x3a, 0x9c, 0xf2, 0x2e, 0x1b, 0x7b, 0x33, 0x9e, 0xb8, 0x63, 0xc2, 0x02, 0x48, 0xa2,
        0xc3, 0x4e, 0x95, 0x98, 0x3d, 0xa3, 0xd4, 0x6c, 0x96, 0xb0, 0x12, 0x4e, 0x6d, 0x69, 0x9c,
        0xc6, 0x2d,
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
        assert_eq!(
            descriptor.descriptor_fingerprint_input().fingerprint(),
            EXPECTED
        );
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
