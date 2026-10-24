//! Nested state owners and generic specialization behind direct adapter aliases.

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
    pub const fn binding_manifest() -> boomerang_runtime::binding::BindingManifest {
        boomerang_runtime::binding::BindingManifest::new(
            boomerang_runtime::binding::DescriptorFingerprint::new([
                183, 233, 254, 229, 118, 90, 34, 77, 92, 30, 45, 77, 65, 5, 171, 12, 202, 203, 236,
                140, 163, 2, 123, 246, 8, 47, 180, 253, 24, 72, 57, 220,
            ]),
            4,
        )
    }
}
