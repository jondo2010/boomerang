use boomerang::prelude::*;

#[reactor(contract = "example.payload", contract_version = 7)]
pub fn r#match(
    #[input] r#async: u32,
    #[output] r#type: u32,
    #[state(default = 0)] r#loop: usize,
) -> impl Reactor {
    reaction! {
        r#move (r#async, startup) r#extern.r#const, r#type -> r#type {
            unreachable!();
        }
    }
    mode! { r#type {
        reaction! {
            (reset) -> history(r#type) {
                unreachable!();
            }
        }
    } }
}

#[cfg(feature = "__boomerang_payload")]
#[test]
fn payload_mode_exports_matching_binding_manifest() {
    use boomerang_builder::{
        compiler::StablePath, DescriptorBounds, DescriptorLifecycle, DescriptorRelationship,
        DescriptorRelationshipKind, DescriptorRelationshipTarget, ModeSlot, ModeSlotId,
        ModeTransitionKind, PortDirection, PortSlot, PortSlotId, ReactionSlot, ReactionSlotId,
        ReactorSlot, ReactorSlotId, StateSlot, StateSlotId,
    };

    let reactor = StablePath::from_name("Match").unwrap();
    let input = reactor.append_name("async").unwrap();
    let output = reactor.append_name("type").unwrap();
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
        "example.payload",
        7,
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
        __boomerang::BINDING_MANIFEST.descriptor_fingerprint(),
        descriptor.descriptor_fingerprint_input().fingerprint(),
    );
    assert_eq!(
        __boomerang::BINDING_MANIFEST.macro_abi(),
        boomerang_builder::COMPONENT_DESCRIPTOR_MACRO_ABI,
    );
}
