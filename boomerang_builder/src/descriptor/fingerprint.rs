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
