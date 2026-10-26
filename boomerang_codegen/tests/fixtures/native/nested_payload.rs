//! Nested state owners and generic specialization behind direct adapter aliases.
//! The existing native fixture verifies these contracts without another Cargo test case.

boomerang::component_payload! {
    pub mod parent {
        use boomerang::prelude::*;
        #[reactor(contract = "example.parent", contract_version = 1,
            bounds(queue_capacity = 4, payload_bytes = 64, state_bytes = 64, scratch_bytes = 64))]
        pub fn Parent(#[state(default = 5)] marker: u32) -> impl Reactor {
            reaction! { start(startup) {
                assert_eq!(state.marker, 5);
                state.marker += 10;
                crate::NESTED.fetch_or(1, std::sync::atomic::Ordering::SeqCst);
            } }
        }
    }
}
boomerang::component_payload! {
    pub mod child {
        use boomerang::prelude::*;
        #[reactor(contract = "example.child", contract_version = 1,
            bounds(queue_capacity = 4, payload_bytes = 64, state_bytes = 64, scratch_bytes = 64))]
        pub fn Child<T, const N: usize>(#[state] marker: T) -> impl Reactor
        where T: boomerang::runtime::ReactorData + Copy + Default + PartialEq {
            reaction! { start(startup) {
                assert!(state.marker == T::default());
                assert_eq!(N, 3);
                crate::NESTED.fetch_or(2, std::sync::atomic::Ordering::SeqCst);
            } }
        }
    }
}

/// Manually composed component exports; generated payload adapters own concrete state types.
#[allow(non_upper_case_globals)]
pub mod exports {
    pub use super::parent::__boomerang::payload::{
        reaction_Parent_2fstart as reaction_Root_2fstart, state_Parent as state_Root,
    };
    use boomerang_runtime::{
        CompiledModeEffectRef, Context, ReactionBindingError, ReactionRefs, ReactorData,
    };
    pub const state_Root_2fchild: fn() -> super::child::ChildState<bool, 3> =
        super::child::__boomerang::payload::state_Child::<bool, 3>;
    pub const reaction_Root_2fchild_2fstart: for<'store> fn(
        &mut Context,
        &mut dyn ReactorData,
        ReactionRefs<'store>,
        Option<CompiledModeEffectRef>,
    )
        -> Result<(), ReactionBindingError> =
        super::child::__boomerang::payload::reaction_Child_2fstart::<bool, 3>;
    /// Independent composite contract; child and root payload declarations remain target-owned.
    pub struct Payload;
    impl boomerang_runtime::binding::ComponentPayload for Payload {
        const DESCRIPTOR_FINGERPRINT: boomerang_runtime::binding::DescriptorFingerprint =
            boomerang_runtime::binding::DescriptorFingerprint::new([
                0x1d, 0x6e, 0x2d, 0xbe, 0xfb, 0x1e, 0xd5, 0xcf, 0xba, 0x16, 0xde, 0x90, 0x64, 0x73,
                0x02, 0x97, 0x9f, 0x5f, 0x4d, 0xde, 0x8e, 0x21, 0x52, 0x8d, 0x38, 0x58, 0x45, 0x08,
                0x57, 0x76, 0x9c, 0x4a,
            ]);
    }
}
