//! Separate-crate compile boundary for direct payload symbols.

/// Descriptor fingerprint generated independently by the host-side fixture.
const EXPECTED_FINGERPRINT: boomerang::runtime::binding::DescriptorFingerprint =
    boomerang::runtime::binding::DescriptorFingerprint::new([
        173, 248, 107, 207, 105, 80, 159, 129, 225, 21, 134, 108, 49, 224, 42, 183, 112, 195,
        43, 150, 102, 68, 163, 191, 240, 50, 132, 133, 213, 59, 136, 241,
    ]);
const _: () = boomerang::runtime::binding::assert_descriptor_fingerprint(
    EXPECTED_FINGERPRINT,
    descriptor_pass::__boomerang::BINDING_MANIFEST.descriptor_fingerprint(),
);

#[cfg(not(feature = "binding-macro-abi-mismatch"))]
/// Macro ABI generated independently by the host-side fixture.
const EXPECTED_MACRO_ABI: u32 = boomerang::runtime::binding::COMPONENT_DESCRIPTOR_MACRO_ABI;
#[cfg(feature = "binding-macro-abi-mismatch")]
/// Deliberately incompatible launcher macro ABI.
const EXPECTED_MACRO_ABI: u32 = boomerang::runtime::binding::COMPONENT_DESCRIPTOR_MACRO_ABI + 1;
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
    boomerang::runtime::CompiledModeEffectRef,
    boomerang::runtime::OutputRef<'store, u32>,
);

/// Typed fixed-array and dynamic-bank references.
type ShapedRefs<'store> = (
    [boomerang::runtime::InputRef<'store, u16>; 2],
    boomerang::runtime::InputBankRef<'store, u64>,
    [boomerang::runtime::OutputRef<'store, u32>; 3],
    boomerang::runtime::OutputBankRef<'store, u8>,
);

/// Adapts owned-storage references and invokes the root typed reaction symbol.
#[allow(dead_code)]
fn bind_match(
    ctx: &mut boomerang::runtime::Context,
    state: &mut dyn boomerang::runtime::ReactorData,
    refs: boomerang::runtime::ReactionRefs<'_>,
    mode_effect: Option<boomerang::runtime::CompiledModeEffectRef>,
) -> Result<(), boomerang::runtime::ReactionBindingError> {
    let state = state
        .downcast_mut::<descriptor_pass::MatchState>()
        .expect("the generated state initializer supplies MatchState");
    let input = refs.ports.partition()?;
    let startup = refs.actions.partition_mut()?;
    let output = refs.ports_mut.partition_mut()?;
    let refs: MoveRefs<'_> = (
        input,
        startup,
        mode_effect.expect("the compiled reaction declares its canonical mode effect"),
        output,
    );
    descriptor_pass::__boomerang::reaction_Match_2fmove(ctx, state, refs);
    Ok(())
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

/// Typed references for the standard-action fixture reaction.
type ActionRefs<'store> = (
    boomerang::runtime::ActionRef<'store, u32>,
    boomerang::runtime::ActionRef<'store, u16>,
    boomerang::runtime::ActionRef<'store, u32>,
    boomerang::runtime::ActionRef<'store, u16>,
);

/// Invokes a reaction whose references include logical and physical actions.
#[allow(dead_code)]
fn bind_actions<'store>(ctx: &mut boomerang::runtime::Context, refs: ActionRefs<'store>) {
    let mut state = descriptor_pass::actions::__boomerang::state_Actions();
    descriptor_pass::actions::__boomerang::reaction_Actions_2fact(ctx, &mut state, refs);
}

const _: boomerang::runtime::PayloadType<u32> =
    descriptor_pass::__boomerang::port_Match_2fasync;
const _: boomerang::runtime::PayloadType<u32> =
    descriptor_pass::actions::__boomerang::action_Actions_2flogical_5fnow;
const _: boomerang::runtime::PayloadType<u16> =
    descriptor_pass::actions::__boomerang::action_Actions_2fphysical_5flater;

#[allow(dead_code)]
fn direct_payload_bindings() -> boomerang::runtime::EnclaveBindings {
    use boomerang::runtime::image::BindingSlotIndex;

    boomerang::runtime::EnclaveBindings::new()
        .bind_state(BindingSlotIndex::new(0), descriptor_pass::__boomerang::state_Match)
        .bind_reaction(BindingSlotIndex::new(3), bind_match)
        .bind_port(
            BindingSlotIndex::new(1),
            descriptor_pass::__boomerang::port_Match_2fasync,
        )
        .bind_action(
            BindingSlotIndex::new(2),
            descriptor_pass::actions::__boomerang::action_Actions_2flogical_5fnow,
        )
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
