//! Optional host-only canonical preimage encoding and BLAKE3 hashing.
//!
//! Encoding v1 tags top-level descriptor sections. Fields in nested fixed-schema records are
//! positional in declaration order.

use super::*;
use boomerang_runtime::binding::DescriptorFingerprint;

const ENCODING_VERSION: u32 = 1;
const CONTRACT_ID: u8 = 0x01;
const CONTRACT_VERSION: u8 = 0x02;
const MACRO_ABI: u8 = 0x03;
const REACTOR_SLOTS: u8 = 0x10;
const PORT_SLOTS: u8 = 0x11;
const ACTION_SLOTS: u8 = 0x12;
const REACTION_SLOTS: u8 = 0x13;
const MODE_SLOTS: u8 = 0x14;
const STATE_SLOTS: u8 = 0x15;
const CODEC_SLOTS: u8 = 0x16;
const RELATIONSHIPS: u8 = 0x17;
const PLACEMENT_GROUPS: u8 = 0x18;
const ENCLAVES: u8 = 0x19;
const BOUNDS: u8 = 0x1a;

/// Private incremental encoder for the descriptor fingerprint byte stream.
struct CanonicalWriter(
    /// BLAKE3 state receiving the canonical bytes.
    blake3::Hasher,
);

impl CanonicalWriter {
    fn new() -> Self {
        Self(blake3::Hasher::new_derive_key(
            "boomerang.component-descriptor",
        ))
    }

    fn finish(self) -> DescriptorFingerprint {
        DescriptorFingerprint::new(*self.0.finalize().as_bytes())
    }

    fn bytes(&mut self, value: &[u8]) {
        self.0.update(value);
    }

    fn u8(&mut self, value: u8) {
        self.bytes(&[value]);
    }

    fn u32(&mut self, value: u32) {
        self.bytes(&value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.bytes(&value.to_le_bytes());
    }

    fn string(&mut self, value: &str) {
        self.u64(u64::try_from(value.len()).expect("string length exceeds u64"));
        self.bytes(value.as_bytes());
    }

    fn slot_id<T>(&mut self, value: &BindingSlotId<T>) {
        self.string(&value.path().to_string());
    }

    fn option_slot_id<T>(&mut self, value: &Option<BindingSlotId<T>>) {
        match value {
            None => self.u8(0),
            Some(value) => {
                self.u8(1);
                self.slot_id(value);
            }
        }
    }

    fn descriptor_bound(&mut self, value: DescriptorBound) {
        match value {
            DescriptorBound::Unknown => self.u8(0),
            DescriptorBound::Known(value) => {
                self.u8(1);
                self.u64(value);
            }
        }
    }
}

impl DescriptorFingerprintInput {
    /// Computes the versioned canonical fingerprint of this descriptor.
    pub fn fingerprint(&self) -> DescriptorFingerprint {
        let mut writer = CanonicalWriter::new();
        writer.u32(ENCODING_VERSION);

        writer.u8(CONTRACT_ID);
        writer.string(&self.contract_id.to_string());
        writer.u8(CONTRACT_VERSION);
        writer.u64(self.contract_version);
        writer.u8(MACRO_ABI);
        writer.u32(self.macro_abi);

        writer.u8(REACTOR_SLOTS);
        writer.u64(u64::try_from(self.reactor_slots.len()).expect("slot count exceeds u64"));
        for slot in &self.reactor_slots {
            writer.slot_id(&slot.id);
            writer.option_slot_id(&slot.parent);
        }
        writer.u8(PORT_SLOTS);
        writer.u64(u64::try_from(self.port_slots.len()).expect("slot count exceeds u64"));
        for slot in &self.port_slots {
            writer.slot_id(&slot.id);
            writer.slot_id(&slot.reactor);
            writer.u8(match slot.direction {
                PortDirection::Input => 0,
                PortDirection::Output => 1,
            });
        }
        writer.u8(ACTION_SLOTS);
        writer.u64(u64::try_from(self.action_slots.len()).expect("slot count exceeds u64"));
        for slot in &self.action_slots {
            writer.slot_id(&slot.id);
            writer.slot_id(&slot.reactor);
        }
        writer.u8(REACTION_SLOTS);
        writer.u64(u64::try_from(self.reaction_slots.len()).expect("slot count exceeds u64"));
        for slot in &self.reaction_slots {
            writer.slot_id(&slot.id);
            writer.slot_id(&slot.reactor);
        }
        writer.u8(MODE_SLOTS);
        writer.u64(u64::try_from(self.mode_slots.len()).expect("slot count exceeds u64"));
        for slot in &self.mode_slots {
            writer.slot_id(&slot.id);
            writer.slot_id(&slot.reactor);
            writer.option_slot_id(&slot.parent);
            writer.u8(u8::from(slot.initial));
        }
        writer.u8(STATE_SLOTS);
        writer.u64(u64::try_from(self.state_slots.len()).expect("slot count exceeds u64"));
        for slot in &self.state_slots {
            writer.slot_id(&slot.id);
            writer.slot_id(&slot.reactor);
        }
        writer.u8(CODEC_SLOTS);
        writer.u64(u64::try_from(self.codec_slots.len()).expect("slot count exceeds u64"));
        for slot in &self.codec_slots {
            writer.slot_id(&slot.id);
        }
        writer.u8(RELATIONSHIPS);
        writer
            .u64(u64::try_from(self.relationships.len()).expect("relationship count exceeds u64"));
        for relationship in &self.relationships {
            writer.slot_id(&relationship.reaction);
            writer.u8(match relationship.kind {
                DescriptorRelationshipKind::Trigger => 0,
                DescriptorRelationshipKind::Use => 1,
                DescriptorRelationshipKind::Effect => 2,
                DescriptorRelationshipKind::Mode => 3,
                DescriptorRelationshipKind::Scope => 4,
            });
            match &relationship.target {
                DescriptorRelationshipTarget::Port(value) => {
                    writer.u8(0);
                    writer.slot_id(value);
                }
                DescriptorRelationshipTarget::Action(value) => {
                    writer.u8(1);
                    writer.slot_id(value);
                }
                DescriptorRelationshipTarget::Mode(value) => {
                    writer.u8(2);
                    writer.slot_id(value);
                }
                DescriptorRelationshipTarget::Lifecycle(value) => {
                    writer.u8(3);
                    writer.u8(match value {
                        DescriptorLifecycle::Startup => 0,
                        DescriptorLifecycle::Shutdown => 1,
                        DescriptorLifecycle::Reset => 2,
                    });
                }
                DescriptorRelationshipTarget::Lexical(value) => {
                    writer.u8(4);
                    writer.string(&value.to_string());
                }
            }
            match relationship.mode_transition {
                None => writer.u8(0),
                Some(ModeTransitionKind::Reset) => {
                    writer.u8(1);
                    writer.u8(0);
                }
                Some(ModeTransitionKind::History) => {
                    writer.u8(1);
                    writer.u8(1);
                }
            }
            writer.u32(relationship.declaration_position);
        }
        writer.u8(PLACEMENT_GROUPS);
        writer.u64(u64::try_from(self.placement_groups.len()).expect("group count exceeds u64"));
        for group in &self.placement_groups {
            writer.slot_id(&group.id);
            writer.option_slot_id(&group.parent);
        }
        writer.u8(ENCLAVES);
        writer.u64(u64::try_from(self.enclaves.len()).expect("enclave count exceeds u64"));
        for enclave in &self.enclaves {
            writer.slot_id(&enclave.id);
            writer.slot_id(&enclave.root);
        }
        writer.u8(BOUNDS);
        writer.descriptor_bound(self.bounds.queue_capacity);
        writer.descriptor_bound(self.bounds.payload_bytes);
        writer.descriptor_bound(self.bounds.state_bytes);
        writer.descriptor_bound(self.bounds.scratch_bytes);

        writer.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXPECTED_V1: DescriptorFingerprint = DescriptorFingerprint::new([
        233, 39, 15, 186, 116, 156, 116, 48, 134, 207, 222, 177, 36, 110, 49, 101, 247, 179, 180,
        30, 37, 226, 64, 102, 126, 207, 204, 77, 212, 170, 19, 57,
    ]);

    /// Controls insertion order for the complete descriptor fixture.
    #[derive(Clone, Copy)]
    enum InputOrder {
        /// Inserts records in their declared order.
        Forward,
        /// Inserts records in reverse declared order.
        Reversed,
    }

    /// Descriptor parts used by fingerprint contract fixtures.
    #[derive(Default)]
    struct FixtureParts {
        /// Reactor slots.
        reactors: Vec<ReactorSlot>,
        /// Port slots.
        ports: Vec<PortSlot>,
        /// Action slots.
        actions: Vec<ActionSlot>,
        /// Reaction slots.
        reactions: Vec<ReactionSlot>,
        /// Mode slots.
        modes: Vec<ModeSlot>,
        /// State slots.
        states: Vec<StateSlot>,
        /// Codec slots.
        codecs: Vec<CodecSlot>,
        /// Structural relationships.
        relationships: Vec<DescriptorRelationship>,
        /// Placement groups.
        groups: Vec<DescriptorPlacementGroup>,
        /// Enclave declarations.
        enclaves: Vec<DescriptorEnclave>,
    }

    fn build_descriptor(
        parts: FixtureParts,
        contract_version: u64,
        macro_abi: u32,
        bounds: DescriptorBounds,
    ) -> ComponentDescriptor {
        ComponentDescriptor::try_new(
            ContractId::new("example.contract").unwrap(),
            contract_version,
            macro_abi,
            parts.reactors,
            parts.ports,
            parts.actions,
            parts.reactions,
            parts.modes,
            parts.states,
            parts.codecs,
            parts.relationships,
            parts.groups,
            parts.enclaves,
            bounds,
        )
        .unwrap()
    }

    fn complete_descriptor(order: InputOrder) -> ComponentDescriptor {
        complete_descriptor_with_contract_version_and_macro_abi(
            order,
            1,
            COMPONENT_DESCRIPTOR_MACRO_ABI,
        )
    }

    fn complete_descriptor_with_contract_version(contract_version: u64) -> ComponentDescriptor {
        complete_descriptor_with_contract_version_and_macro_abi(
            InputOrder::Forward,
            contract_version,
            COMPONENT_DESCRIPTOR_MACRO_ABI,
        )
    }

    fn complete_descriptor_with_macro_abi(macro_abi: u32) -> ComponentDescriptor {
        complete_descriptor_with_contract_version_and_macro_abi(InputOrder::Forward, 1, macro_abi)
    }

    fn complete_descriptor_with_contract_version_and_macro_abi(
        order: InputOrder,
        contract_version: u64,
        macro_abi: u32,
    ) -> ComponentDescriptor {
        let root = ReactorSlotId::new("Root").unwrap();
        let child = ReactorSlotId::new("Root/child").unwrap();
        let input = PortSlotId::new("Root/input").unwrap();
        let output = PortSlotId::new("Root/output").unwrap();
        let action = ActionSlotId::new("Root/action").unwrap();
        let reaction = ReactionSlotId::new("Root/reaction").unwrap();
        let mode = ModeSlotId::new("Root/mode").unwrap();
        let state = StateSlotId::new("Root/state").unwrap();
        let mut parts = FixtureParts {
            reactors: vec![
                ReactorSlot {
                    id: child,
                    parent: Some(root.clone()),
                },
                ReactorSlot {
                    id: root.clone(),
                    parent: None,
                },
            ],
            ports: vec![
                PortSlot {
                    id: output,
                    reactor: root.clone(),
                    direction: PortDirection::Output,
                },
                PortSlot {
                    id: input.clone(),
                    reactor: root.clone(),
                    direction: PortDirection::Input,
                },
            ],
            actions: vec![ActionSlot {
                id: action.clone(),
                reactor: root.clone(),
            }],
            reactions: vec![ReactionSlot {
                id: reaction.clone(),
                reactor: root.clone(),
            }],
            modes: vec![ModeSlot {
                id: mode.clone(),
                reactor: root.clone(),
                parent: None,
                initial: true,
            }],
            states: vec![StateSlot {
                id: state,
                reactor: root.clone(),
            }],
            codecs: vec![CodecSlot {
                id: CodecSlotId::new("codec").unwrap(),
            }],
            relationships: vec![
                DescriptorRelationship {
                    reaction: reaction.clone(),
                    kind: DescriptorRelationshipKind::Effect,
                    target: DescriptorRelationshipTarget::Mode(mode),
                    mode_transition: Some(ModeTransitionKind::History),
                    declaration_position: 1,
                },
                DescriptorRelationship {
                    reaction: reaction.clone(),
                    kind: DescriptorRelationshipKind::Trigger,
                    target: DescriptorRelationshipTarget::Port(input),
                    mode_transition: None,
                    declaration_position: 0,
                },
                DescriptorRelationship {
                    reaction: reaction.clone(),
                    kind: DescriptorRelationshipKind::Use,
                    target: DescriptorRelationshipTarget::Action(action),
                    mode_transition: Some(ModeTransitionKind::Reset),
                    declaration_position: 2,
                },
                DescriptorRelationship {
                    reaction: reaction.clone(),
                    kind: DescriptorRelationshipKind::Scope,
                    target: DescriptorRelationshipTarget::Lexical(
                        StablePath::from_name("lexical").unwrap(),
                    ),
                    mode_transition: None,
                    declaration_position: 3,
                },
                DescriptorRelationship {
                    reaction,
                    kind: DescriptorRelationshipKind::Mode,
                    target: DescriptorRelationshipTarget::Lifecycle(DescriptorLifecycle::Shutdown),
                    mode_transition: None,
                    declaration_position: 4,
                },
            ],
            groups: vec![DescriptorPlacementGroup {
                id: PlacementGroupSlotId::new("group").unwrap(),
                parent: None,
            }],
            enclaves: vec![DescriptorEnclave {
                id: EnclaveSlotId::new("enclave").unwrap(),
                root,
            }],
        };
        if matches!(order, InputOrder::Reversed) {
            parts.reactors.reverse();
            parts.ports.reverse();
            parts.actions.reverse();
            parts.reactions.reverse();
            parts.modes.reverse();
            parts.states.reverse();
            parts.codecs.reverse();
            parts.relationships.reverse();
            parts.groups.reverse();
            parts.enclaves.reverse();
        }
        build_descriptor(
            parts,
            contract_version,
            macro_abi,
            DescriptorBounds {
                queue_capacity: DescriptorBound::Known(1),
                payload_bytes: DescriptorBound::Known(2),
                state_bytes: DescriptorBound::Unknown,
                scratch_bytes: DescriptorBound::Known(4),
            },
        )
    }

    fn descriptor_with_port_direction(direction: PortDirection) -> ComponentDescriptor {
        let root = ReactorSlotId::new("Root").unwrap();
        build_descriptor(
            FixtureParts {
                reactors: vec![ReactorSlot {
                    id: root.clone(),
                    parent: None,
                }],
                ports: vec![PortSlot {
                    id: PortSlotId::new("Root/value").unwrap(),
                    reactor: root,
                    direction,
                }],
                ..FixtureParts::default()
            },
            1,
            COMPONENT_DESCRIPTOR_MACRO_ABI,
            DescriptorBounds::default(),
        )
    }

    fn descriptor_with_ports(names: &[&str]) -> ComponentDescriptor {
        let root = ReactorSlotId::new("Root").unwrap();
        build_descriptor(
            FixtureParts {
                reactors: vec![ReactorSlot {
                    id: root.clone(),
                    parent: None,
                }],
                ports: names
                    .iter()
                    .map(|name| PortSlot {
                        id: PortSlotId::new(format!("Root/{name}")).unwrap(),
                        reactor: root.clone(),
                        direction: PortDirection::Input,
                    })
                    .collect(),
                ..FixtureParts::default()
            },
            1,
            COMPONENT_DESCRIPTOR_MACRO_ABI,
            DescriptorBounds::default(),
        )
    }

    #[test]
    fn canonical_order_is_input_order_insensitive() {
        let left = descriptor_with_ports(&["z", "a"]);
        let right = descriptor_with_ports(&["a", "z"]);
        assert_eq!(left.port_slots()[0].id.to_string(), "Root/a");
        assert_eq!(
            left.descriptor_fingerprint_input(),
            right.descriptor_fingerprint_input()
        );
        assert!(std::ptr::eq(
            left.contract_id(),
            left.descriptor_fingerprint_input().contract_id()
        ));
        assert!(std::ptr::eq(
            left.port_slots(),
            left.descriptor_fingerprint_input().port_slots()
        ));
    }

    #[test]
    fn descriptor_fingerprint_is_canonical_and_semantically_sensitive() {
        let forward = complete_descriptor(InputOrder::Forward);
        let reversed = complete_descriptor(InputOrder::Reversed);
        let fingerprint = forward.descriptor_fingerprint_input().fingerprint();
        assert_eq!(fingerprint, EXPECTED_V1);
        assert_eq!(
            fingerprint,
            reversed.descriptor_fingerprint_input().fingerprint(),
        );
        assert_ne!(
            fingerprint,
            complete_descriptor_with_contract_version(2)
                .descriptor_fingerprint_input()
                .fingerprint(),
        );
        assert_ne!(
            fingerprint,
            complete_descriptor_with_macro_abi(COMPONENT_DESCRIPTOR_MACRO_ABI + 1)
                .descriptor_fingerprint_input()
                .fingerprint(),
        );
    }

    #[test]
    fn descriptor_fingerprint_changes_with_port_direction() {
        assert_ne!(
            descriptor_with_port_direction(PortDirection::Input)
                .descriptor_fingerprint_input()
                .fingerprint(),
            descriptor_with_port_direction(PortDirection::Output)
                .descriptor_fingerprint_input()
                .fingerprint(),
        );
    }
}
