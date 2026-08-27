use boomerang::prelude::*;

#[reactor(contract = "example.sensor", contract_version = 1)]
pub fn r#match(
    #[input] r#async: u32,
    #[output] r#type: u32,
    #[output] r#yield: u32,
    #[state(default = state_constructor_must_not_compile())] r#loop: usize,
) -> impl Reactor {
    reaction! {
        r#move (r#async, startup) r#extern.r#const, r#type -> r#type, r#yield {
            reaction_payload_must_not_compile();
        }
    }
    mode! { r#type {
        reaction! {
            (reset) -> history(r#type) {
                reaction_payload_must_not_compile();
            }
        }
    } }
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
        assert_eq!(descriptor.relationships().len(), 9);
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
        let lexical = reactor
            .append_name("extern")
            .unwrap()
            .append_name("const")
            .unwrap();
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
                    kind: DescriptorRelationshipKind::Use,
                    target: DescriptorRelationshipTarget::Lexical(lexical),
                    mode_transition: None,
                    declaration_position: 0,
                },
                DescriptorRelationship {
                    reaction: named_reaction_id.clone(),
                    kind: DescriptorRelationshipKind::Use,
                    target: DescriptorRelationshipTarget::Mode(mode_id.clone()),
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
    }
}

#[cfg(all(
    feature = "__boomerang_payload",
    feature = "binding-fingerprint-mismatch"
))]
const _: () = boomerang::runtime::binding::assert_descriptor_fingerprint(
    boomerang::runtime::binding::DescriptorFingerprint::new([0; 32]),
    __boomerang::BINDING_MANIFEST.descriptor_fingerprint(),
);
