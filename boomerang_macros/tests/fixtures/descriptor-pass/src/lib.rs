use boomerang::prelude::*;

#[cfg(not(feature = "__boomerang_descriptor"))]
fn initial_count() -> usize {
    3
}

#[cfg(not(feature = "__boomerang_descriptor"))]
fn target_only_reaction_payload() {}

#[reactor(contract = "example.sensor", contract_version = 1)]
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

pub mod custom {
    //! Custom-state payload export fixture.

    use super::*;

    /// State constructed through an explicit `state_init` path.
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct CustomState {
        /// Initial count observed by the payload test.
        pub count: usize,
    }

    #[cfg(feature = "__boomerang_payload")]
    fn init_custom_state() -> CustomState {
        CustomState { count: 11 }
    }

    #[reactor(
        contract = "example.custom",
        contract_version = 1,
        state = CustomState,
        state_init = init_custom_state
    )]
    pub fn Custom() -> impl Reactor {
        reaction! {
            start (startup) {
                state.count += 1;
            }
        }
    }
}

pub mod shaped {
    //! Fixed-array and dynamic-bank payload reference fixture.

    use super::*;

    /// Declares every currently supported own-port reference shape.
    #[reactor(contract = "example.shaped", contract_version = 1)]
    pub fn Shaped(
        #[input] array_in: [u16; 2],
        #[input(len = 2)] bank_in: u64,
        #[output] array_out: [u32; 3],
        #[output(len = 3)] bank_out: u8,
    ) -> impl Reactor {
        reaction! {
            shaped (array_in, bank_in) -> array_out, bank_out {}
        }
    }
}

#[cfg(feature = "missing-state-init")]
mod missing_state_init {
    use super::*;

    #[derive(Clone)]
    struct CustomState;

    #[reactor(
        contract = "example.missing-state-init",
        contract_version = 1,
        state = CustomState
    )]
    fn MissingStateInit() -> impl Reactor {}
}

#[cfg(feature = "payload-lexical-relation")]
mod lexical_relation {
    use super::*;

    #[reactor(contract = "example.lexical", contract_version = 1)]
    fn LexicalRelation() -> impl Reactor {
        reaction! {
            (child.output) {
                let _ = child_output;
            }
        }
    }
}

#[cfg(feature = "__boomerang_descriptor")]
pub fn descriptor() -> boomerang::builder::ComponentDescriptor {
    __boomerang::descriptor()
}

#[cfg(all(test, feature = "__boomerang_descriptor"))]
mod descriptor_tests {
    #[test]
    fn descriptor_contains_only_source_observable_structure() {
        let descriptor = super::descriptor();
        assert_eq!(descriptor.contract_id().as_str(), "example.sensor");
        assert_eq!(descriptor.contract_version(), 1);
        assert_eq!(descriptor.reactor_slots().len(), 1);
        assert_eq!(descriptor.reactor_slots()[0].id.to_string(), "Match");
        assert_eq!(descriptor.port_slots().len(), 3);
        assert_eq!(descriptor.port_slots()[0].id.to_string(), "Match/async");
        assert_eq!(descriptor.port_slots()[1].id.to_string(), "Match/type");
        assert_eq!(descriptor.port_slots()[2].id.to_string(), "Match/yield");
        assert_eq!(descriptor.reaction_slots().len(), 2);
        assert_eq!(descriptor.reaction_slots()[0].id.to_string(), "Match/move");
        assert_eq!(descriptor.reaction_slots()[1].id.to_string(), "Match/#g1");
        assert_eq!(descriptor.mode_slots()[0].id.to_string(), "Match/type");
        assert_eq!(descriptor.relationships().len(), 7);
        assert_eq!(descriptor.state_slots().len(), 1);
        assert_eq!(descriptor.state_slots()[0].id.to_string(), "Match/loop");
        assert!(descriptor.codec_slots().is_empty());
        assert!(descriptor.placement_groups().is_empty());
        assert!(descriptor.enclaves().is_empty());
        assert_eq!(
            descriptor.descriptor_fingerprint_input().state_slots(),
            descriptor.state_slots()
        );
    }
}

#[cfg(feature = "__boomerang_payload")]
mod launcher_like {
    const EXPECTED_FINGERPRINT: boomerang::runtime::binding::DescriptorFingerprint =
        boomerang::runtime::binding::DescriptorFingerprint::new([
            199, 47, 226, 90, 173, 60, 225, 154, 76, 243, 98, 167, 187, 152, 112, 172, 14, 8, 133,
            30, 202, 200, 210, 248, 170, 205, 229, 200, 32, 15, 101, 153,
        ]);
    const _: () = boomerang::runtime::binding::assert_descriptor_fingerprint(
        EXPECTED_FINGERPRINT,
        super::__boomerang::BINDING_MANIFEST.descriptor_fingerprint(),
    );

    #[cfg(not(feature = "binding-macro-abi-mismatch"))]
    const EXPECTED_MACRO_ABI: u32 = boomerang_builder::COMPONENT_DESCRIPTOR_MACRO_ABI;
    #[cfg(feature = "binding-macro-abi-mismatch")]
    const EXPECTED_MACRO_ABI: u32 = boomerang_builder::COMPONENT_DESCRIPTOR_MACRO_ABI + 1;
    const _: () = assert!(
        EXPECTED_MACRO_ABI == super::__boomerang::BINDING_MANIFEST.macro_abi(),
        "macro ABI mismatch",
    );

    #[cfg(feature = "binding-fingerprint-mismatch")]
    const _: () = boomerang::runtime::binding::assert_descriptor_fingerprint(
        boomerang::runtime::binding::DescriptorFingerprint::new([0; 32]),
        super::__boomerang::BINDING_MANIFEST.descriptor_fingerprint(),
    );

    type MoveRefs<'store> = (
        boomerang::runtime::InputRef<'store, u32>,
        boomerang::runtime::ActionRef<'store>,
        boomerang::runtime::ModeEffectRef,
        boomerang::runtime::OutputRef<'store, u32>,
    );
    type ShapedRefs<'store> = (
        [boomerang::runtime::InputRef<'store, u16>; 2],
        boomerang::runtime::InputBankRef<'store, u64>,
        [boomerang::runtime::OutputRef<'store, u32>; 3],
        boomerang::runtime::OutputBankRef<'store, u8>,
    );

    #[allow(dead_code)]
    fn bind_match<'store>(ctx: &mut boomerang::runtime::Context, refs: MoveRefs<'store>) {
        let mut state = super::__boomerang::state_Match();
        super::__boomerang::reaction_Match_2fmove(ctx, &mut state, refs);
    }

    #[allow(dead_code)]
    fn bind_shaped<'store>(ctx: &mut boomerang::runtime::Context, refs: ShapedRefs<'store>) {
        let mut state = super::shaped::__boomerang::state_Shaped();
        super::shaped::__boomerang::reaction_Shaped_2fshaped(ctx, &mut state, refs);
    }

    #[allow(dead_code)]
    fn bind_custom<'store>(
        ctx: &mut boomerang::runtime::Context,
        refs: (boomerang::runtime::ActionRef<'store>,),
    ) {
        let mut state = super::custom::__boomerang::state_Custom();
        super::custom::__boomerang::reaction_Custom_2fstart(ctx, &mut state, refs);
    }
}

#[cfg(all(test, feature = "__boomerang_payload"))]
mod payload_tests {
    use boomerang_builder::{
        compiler::StablePath, DescriptorBounds, DescriptorLifecycle, DescriptorRelationship,
        DescriptorRelationshipKind, DescriptorRelationshipTarget, ModeSlot, ModeSlotId,
        ModeTransitionKind, PortDirection, PortSlot, PortSlotId, ReactionSlot, ReactionSlotId,
        ReactorSlot, ReactorSlotId, StateSlot, StateSlotId,
    };

    #[test]
    fn payload_mode_exports_matching_binding_manifest() {
        let reactor = StablePath::from_name("Match").unwrap();
        let input = reactor.append_name("async").unwrap();
        let output = reactor.append_name("type").unwrap();
        let effect = reactor.append_name("yield").unwrap();
        let state = reactor.append_name("loop").unwrap();
        let mode = reactor.append_name("type").unwrap();
        let named_reaction = reactor.append_name("move").unwrap();
        let anonymous_reaction = reactor.append_generated_ordinal(1);
        let reactor_id = ReactorSlotId::from_path(reactor.clone());
        let named_reaction_id = ReactionSlotId::from_path(named_reaction);
        let anonymous_reaction_id = ReactionSlotId::from_path(anonymous_reaction);
        let mode_id = ModeSlotId::from_path(mode);

        let descriptor = boomerang_builder::ComponentDescriptor::__from_macro(
            "example.sensor",
            1,
            boomerang_builder::COMPONENT_DESCRIPTOR_MACRO_ABI,
            vec![ReactorSlot {
                id: reactor_id.clone(),
                parent: None,
            }],
            vec![
                PortSlot {
                    id: PortSlotId::from_path(input.clone()),
                    reactor: reactor_id.clone(),
                    direction: PortDirection::Input,
                },
                PortSlot {
                    id: PortSlotId::from_path(output),
                    reactor: reactor_id.clone(),
                    direction: PortDirection::Output,
                },
                PortSlot {
                    id: PortSlotId::from_path(effect.clone()),
                    reactor: reactor_id.clone(),
                    direction: PortDirection::Output,
                },
            ],
            vec![],
            vec![
                ReactionSlot {
                    id: named_reaction_id.clone(),
                    reactor: reactor_id.clone(),
                },
                ReactionSlot {
                    id: anonymous_reaction_id.clone(),
                    reactor: reactor_id.clone(),
                },
            ],
            vec![ModeSlot {
                id: mode_id.clone(),
                reactor: reactor_id.clone(),
                parent: None,
                initial: false,
            }],
            vec![StateSlot {
                id: StateSlotId::from_path(state),
                reactor: reactor_id.clone(),
            }],
            vec![],
            vec![
                DescriptorRelationship {
                    reaction: named_reaction_id.clone(),
                    kind: DescriptorRelationshipKind::Trigger,
                    target: DescriptorRelationshipTarget::Port(PortSlotId::from_path(input)),
                    mode_transition: None,
                    declaration_position: 0,
                },
                DescriptorRelationship {
                    reaction: named_reaction_id.clone(),
                    kind: DescriptorRelationshipKind::Trigger,
                    target: DescriptorRelationshipTarget::Lifecycle(DescriptorLifecycle::Startup),
                    mode_transition: None,
                    declaration_position: 1,
                },
                DescriptorRelationship {
                    reaction: named_reaction_id.clone(),
                    kind: DescriptorRelationshipKind::Mode,
                    target: DescriptorRelationshipTarget::Mode(mode_id.clone()),
                    mode_transition: Some(ModeTransitionKind::Reset),
                    declaration_position: 0,
                },
                DescriptorRelationship {
                    reaction: named_reaction_id.clone(),
                    kind: DescriptorRelationshipKind::Effect,
                    target: DescriptorRelationshipTarget::Port(PortSlotId::from_path(effect)),
                    mode_transition: None,
                    declaration_position: 1,
                },
                DescriptorRelationship {
                    reaction: anonymous_reaction_id.clone(),
                    kind: DescriptorRelationshipKind::Trigger,
                    target: DescriptorRelationshipTarget::Lifecycle(DescriptorLifecycle::Reset),
                    mode_transition: None,
                    declaration_position: 0,
                },
                DescriptorRelationship {
                    reaction: anonymous_reaction_id.clone(),
                    kind: DescriptorRelationshipKind::Mode,
                    target: DescriptorRelationshipTarget::Mode(mode_id.clone()),
                    mode_transition: Some(ModeTransitionKind::History),
                    declaration_position: 0,
                },
                DescriptorRelationship {
                    reaction: anonymous_reaction_id,
                    kind: DescriptorRelationshipKind::Scope,
                    target: DescriptorRelationshipTarget::Mode(mode_id),
                    mode_transition: None,
                    declaration_position: 0,
                },
            ],
            vec![],
            vec![],
            DescriptorBounds::default(),
        );

        assert_eq!(
            super::__boomerang::BINDING_MANIFEST.descriptor_fingerprint(),
            descriptor.descriptor_fingerprint_input().fingerprint(),
        );
        assert_eq!(
            super::__boomerang::BINDING_MANIFEST.macro_abi(),
            boomerang_builder::COMPONENT_DESCRIPTOR_MACRO_ABI,
        );

        let state: super::MatchState = super::__boomerang::state_Match();
        assert_eq!(state.r#loop, 3);
        let custom_state: super::custom::CustomState = super::custom::__boomerang::state_Custom();
        assert_eq!(custom_state.count, 11);
    }
}
