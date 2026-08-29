//! Separate-crate compile boundary for direct payload symbols.

/// Descriptor fingerprint generated independently by the host-side fixture.
const EXPECTED_FINGERPRINT: boomerang::runtime::binding::DescriptorFingerprint =
    boomerang::runtime::binding::DescriptorFingerprint::new([
        199, 47, 226, 90, 173, 60, 225, 154, 76, 243, 98, 167, 187, 152, 112, 172, 14, 8, 133, 30,
        202, 200, 210, 248, 170, 205, 229, 200, 32, 15, 101, 153,
    ]);
const _: () = boomerang::runtime::binding::assert_descriptor_fingerprint(
    EXPECTED_FINGERPRINT,
    descriptor_pass::__boomerang::BINDING_MANIFEST.descriptor_fingerprint(),
);

#[cfg(not(feature = "binding-macro-abi-mismatch"))]
/// Macro ABI generated independently by the host-side fixture.
const EXPECTED_MACRO_ABI: u32 = boomerang_builder::COMPONENT_DESCRIPTOR_MACRO_ABI;
#[cfg(feature = "binding-macro-abi-mismatch")]
/// Deliberately incompatible launcher macro ABI.
const EXPECTED_MACRO_ABI: u32 = boomerang_builder::COMPONENT_DESCRIPTOR_MACRO_ABI + 1;
const _: () = assert!(
    EXPECTED_MACRO_ABI == descriptor_pass::__boomerang::BINDING_MANIFEST.macro_abi(),
    "macro ABI mismatch",
);

#[cfg(feature = "binding-fingerprint-mismatch")]
const _: () = boomerang::runtime::binding::assert_descriptor_fingerprint(
    boomerang::runtime::binding::DescriptorFingerprint::new([0; 32]),
    descriptor_pass::__boomerang::BINDING_MANIFEST.descriptor_fingerprint(),
);

/// Typed references for the root implementation reaction.
type MoveRefs<'store> = (
    boomerang::runtime::InputRef<'store, u32>,
    boomerang::runtime::ActionRef<'store>,
    boomerang::runtime::ModeEffectRef,
    boomerang::runtime::OutputRef<'store, u32>,
);

/// Typed fixed-array and dynamic-bank references.
type ShapedRefs<'store> = (
    [boomerang::runtime::InputRef<'store, u16>; 2],
    boomerang::runtime::InputBankRef<'store, u64>,
    [boomerang::runtime::OutputRef<'store, u32>; 3],
    boomerang::runtime::OutputBankRef<'store, u8>,
);

/// Directly constructs root state and invokes its typed reaction symbol.
#[allow(dead_code)]
fn bind_match<'store>(ctx: &mut boomerang::runtime::Context, refs: MoveRefs<'store>) {
    let mut state = descriptor_pass::__boomerang::state_Match();
    descriptor_pass::__boomerang::reaction_Match_2fmove(ctx, &mut state, refs);
}

/// Directly constructs shaped state and invokes its typed reaction symbol.
#[allow(dead_code)]
fn bind_shaped<'store>(ctx: &mut boomerang::runtime::Context, refs: ShapedRefs<'store>) {
    let mut state = descriptor_pass::shaped::__boomerang::state_Shaped();
    descriptor_pass::shaped::__boomerang::reaction_Shaped_2fshaped(ctx, &mut state, refs);
}

/// Directly constructs custom state and invokes its typed reaction symbol.
#[allow(dead_code)]
fn bind_custom<'store>(
    ctx: &mut boomerang::runtime::Context,
    refs: (boomerang::runtime::ActionRef<'store>,),
) {
    let mut state = descriptor_pass::custom::__boomerang::state_Custom();
    descriptor_pass::custom::__boomerang::reaction_Custom_2fstart(ctx, &mut state, refs);
}

/// Directly uses state and reaction symbols from a private generic reactor.
#[allow(dead_code)]
fn bind_lifetime<'store>(
    ctx: &mut boomerang::runtime::Context,
    refs: (boomerang::runtime::ActionRef<'store>,),
) {
    let mut state = descriptor_pass::lifetime_collision::__boomerang::state_Lifetime();
    descriptor_pass::lifetime_collision::__boomerang::reaction_Lifetime_2ftick(
        ctx, &mut state, refs,
    );
}

/// Keeps a private reactor's generated state type nameable by the launcher.
#[allow(dead_code)]
fn generated_state_is_public(
    state: descriptor_pass::lifetime_collision::LifetimeState<'static>,
) -> &'static str {
    state.marker
}

/// Keeps a private reactor's generated empty state alias nameable by the launcher.
#[allow(dead_code)]
fn generated_empty_state_is_public(
    state: descriptor_pass::private_empty::EmptyState,
) -> descriptor_pass::private_empty::EmptyState {
    state
}

/// Keeps the custom state type nameable from the launcher crate.
#[allow(dead_code)]
fn custom_state_is_public(state: descriptor_pass::custom::CustomState) -> usize {
    state.count
}
