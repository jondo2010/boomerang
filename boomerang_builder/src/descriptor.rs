//! Canonical host-owned component descriptor records.

use crate::compiler::{BindingSlotId, ContractId, PlacementGroupId, StableEnclaveId, StablePath};

/// Macro ABI understood by descriptors emitted from this crate version.
pub const COMPONENT_DESCRIPTOR_MACRO_ABI: u32 = 1;

/// Marker for stable reactor binding slots.
pub enum ReactorSlotMarker {}
/// Marker for stable port binding slots.
pub enum PortSlotMarker {}
/// Marker for stable action binding slots.
pub enum ActionSlotMarker {}
/// Marker for stable reaction binding slots.
pub enum ReactionSlotMarker {}
/// Marker for stable mode binding slots.
pub enum ModeSlotMarker {}
/// Marker for stable state binding slots.
pub enum StateSlotMarker {}
/// Marker for stable codec binding slots.
pub enum CodecSlotMarker {}

/// Stable implementation-local reactor slot identity.
pub type ReactorSlotId = BindingSlotId<ReactorSlotMarker>;
/// Stable implementation-local port slot identity.
pub type PortSlotId = BindingSlotId<PortSlotMarker>;
/// Stable implementation-local action slot identity.
pub type ActionSlotId = BindingSlotId<ActionSlotMarker>;
/// Stable implementation-local reaction slot identity.
pub type ReactionSlotId = BindingSlotId<ReactionSlotMarker>;
/// Stable implementation-local mode slot identity.
pub type ModeSlotId = BindingSlotId<ModeSlotMarker>;
/// Stable implementation-local state slot identity.
pub type StateSlotId = BindingSlotId<StateSlotMarker>;
/// Stable implementation-local codec slot identity.
pub type CodecSlotId = BindingSlotId<CodecSlotMarker>;

/// Stable reactor slot and its structural parent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReactorSlot {
    /// Stable implementation-local identity.
    pub id: ReactorSlotId,
    /// Optional parent reactor slot.
    pub parent: Option<ReactorSlotId>,
}

/// Structural direction of a descriptor port slot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DescriptorPortDirection {
    /// Input received by its reactor.
    Input,
    /// Output produced by its reactor.
    Output,
}

/// Stable port slot owned by one reactor slot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortSlot {
    /// Stable implementation-local identity.
    pub id: PortSlotId,
    /// Owning reactor slot.
    pub reactor: ReactorSlotId,
    /// Structural port direction.
    pub direction: DescriptorPortDirection,
}

/// Stable action slot owned by one reactor slot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionSlot {
    /// Stable implementation-local identity.
    pub id: ActionSlotId,
    /// Owning reactor slot.
    pub reactor: ReactorSlotId,
}

/// Stable reaction slot owned by one reactor slot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReactionSlot {
    /// Stable implementation-local identity.
    pub id: ReactionSlotId,
    /// Owning reactor slot.
    pub reactor: ReactorSlotId,
}

/// Stable mode slot owned by one reactor slot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModeSlot {
    /// Stable implementation-local identity.
    pub id: ModeSlotId,
    /// Owning reactor slot.
    pub reactor: ReactorSlotId,
    /// Optional parent mode slot.
    pub parent: Option<ModeSlotId>,
    /// Whether this is the initial sibling mode.
    pub initial: bool,
}

/// Stable state binding slot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StateSlot {
    /// Stable implementation-local identity.
    pub id: StateSlotId,
    /// Owning reactor slot.
    pub reactor: ReactorSlotId,
}

/// Stable payload codec binding slot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodecSlot {
    /// Stable implementation-local identity.
    pub id: CodecSlotId,
}

/// Source-observable lifecycle trigger.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum DescriptorLifecycle {
    /// Reactor startup.
    Startup,
    /// Reactor shutdown.
    Shutdown,
    /// Entry into a reset mode scope.
    Reset,
}

/// Stable target of a descriptor relationship.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum DescriptorRelationshipTarget {
    /// Stable port slot.
    Port(PortSlotId),
    /// Stable action slot.
    Action(ActionSlotId),
    /// Stable mode slot.
    Mode(ModeSlotId),
    /// Built-in lifecycle trigger.
    Lifecycle(DescriptorLifecycle),
    /// Lexical target whose slot category is resolved by later structural binding.
    Lexical(StablePath),
}

/// Semantic role of one reaction relationship.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum DescriptorRelationshipKind {
    /// Target triggers the reaction.
    Trigger,
    /// Reaction reads the target.
    Use,
    /// Reaction writes, schedules, or transitions the target.
    Effect,
    /// Target controls modal enablement or transition behavior.
    Mode,
    /// Target lexically contains the reaction.
    Scope,
}

/// Modal transition carried by a mode relationship.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum DescriptorModeTransition {
    /// Reset target mode state on entry.
    Reset,
    /// Restore target mode history on entry.
    History,
}

/// One canonical relationship from a reaction to a stable target.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DescriptorRelationship {
    /// Source reaction slot.
    pub reaction: ReactionSlotId,
    /// Relationship role.
    pub kind: DescriptorRelationshipKind,
    /// Stable relationship target.
    pub target: DescriptorRelationshipTarget,
    /// Optional modal transition semantics.
    pub mode_transition: Option<DescriptorModeTransition>,
    /// Zero-based position within this relationship category.
    pub declaration_position: u32,
}

/// Source-declared placement group visible to deployment selection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DescriptorPlacementGroup {
    /// Stable placement-group identity.
    pub id: PlacementGroupId,
    /// Optional parent placement group.
    pub parent: Option<PlacementGroupId>,
}

/// Source-declared scheduler and logical-time domain.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DescriptorEnclave {
    /// Stable Enclave identity.
    pub id: StableEnclaveId,
    /// Stable root reactor slot.
    pub root: ReactorSlotId,
}

/// A deployability bound that may not yet be expressed by source syntax.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DescriptorBound {
    /// Source syntax did not declare this bound.
    Unknown,
    /// Declared inclusive upper bound.
    Known(u64),
}

/// Declared storage and queue bounds for one component implementation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DescriptorBounds {
    /// Maximum queued events.
    pub queue_capacity: DescriptorBound,
    /// Maximum payload storage in bytes.
    pub payload_bytes: DescriptorBound,
    /// Maximum state storage in bytes.
    pub state_bytes: DescriptorBound,
    /// Maximum scratch storage in bytes.
    pub scratch_bytes: DescriptorBound,
}

impl Default for DescriptorBounds {
    fn default() -> Self {
        Self {
            queue_capacity: DescriptorBound::Unknown,
            payload_bytes: DescriptorBound::Unknown,
            state_bytes: DescriptorBound::Unknown,
            scratch_bytes: DescriptorBound::Unknown,
        }
    }
}

/// Canonically ordered typed data supplied to a future fingerprint encoder.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DescriptorFingerprintInput {
    /// Stable external contract identity.
    pub contract_id: ContractId,
    /// Stable external contract version.
    pub contract_version: u64,
    /// Descriptor macro ABI version.
    pub macro_abi: u32,
    /// Canonically ordered reactor slots.
    pub reactor_slots: Box<[ReactorSlot]>,
    /// Canonically ordered port slots.
    pub port_slots: Box<[PortSlot]>,
    /// Canonically ordered action slots.
    pub action_slots: Box<[ActionSlot]>,
    /// Canonically ordered reaction slots.
    pub reaction_slots: Box<[ReactionSlot]>,
    /// Canonically ordered mode slots.
    pub mode_slots: Box<[ModeSlot]>,
    /// Canonically ordered state slots.
    pub state_slots: Box<[StateSlot]>,
    /// Canonically ordered codec slots.
    pub codec_slots: Box<[CodecSlot]>,
    /// Canonically ordered reaction relationships.
    pub relationships: Box<[DescriptorRelationship]>,
    /// Canonically ordered placement groups.
    pub placement_groups: Box<[DescriptorPlacementGroup]>,
    /// Canonically ordered Enclaves.
    pub enclaves: Box<[DescriptorEnclave]>,
    /// Declared resource bounds.
    pub bounds: DescriptorBounds,
}

/// Host-owned structural descriptor generated for one component implementation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComponentDescriptor {
    /// Stable external contract identity.
    pub contract_id: ContractId,
    /// Stable external contract version.
    pub contract_version: u64,
    /// Descriptor macro ABI version.
    pub macro_abi: u32,
    /// Canonically ordered reactor slots.
    pub reactor_slots: Box<[ReactorSlot]>,
    /// Canonically ordered port slots.
    pub port_slots: Box<[PortSlot]>,
    /// Canonically ordered action slots.
    pub action_slots: Box<[ActionSlot]>,
    /// Canonically ordered reaction slots.
    pub reaction_slots: Box<[ReactionSlot]>,
    /// Canonically ordered mode slots.
    pub mode_slots: Box<[ModeSlot]>,
    /// Canonically ordered state slots.
    pub state_slots: Box<[StateSlot]>,
    /// Canonically ordered codec slots.
    pub codec_slots: Box<[CodecSlot]>,
    /// Canonically ordered reaction relationships.
    pub relationships: Box<[DescriptorRelationship]>,
    /// Canonically ordered placement groups.
    pub placement_groups: Box<[DescriptorPlacementGroup]>,
    /// Canonically ordered Enclaves.
    pub enclaves: Box<[DescriptorEnclave]>,
    /// Declared resource bounds.
    pub bounds: DescriptorBounds,
    /// Typed canonical input for descriptor fingerprinting.
    pub descriptor_fingerprint_input: DescriptorFingerprintInput,
}

impl ComponentDescriptor {
    /// Constructs and canonically orders one generated descriptor.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        contract_id: ContractId,
        contract_version: u64,
        macro_abi: u32,
        mut reactor_slots: Vec<ReactorSlot>,
        mut port_slots: Vec<PortSlot>,
        mut action_slots: Vec<ActionSlot>,
        mut reaction_slots: Vec<ReactionSlot>,
        mut mode_slots: Vec<ModeSlot>,
        mut state_slots: Vec<StateSlot>,
        mut codec_slots: Vec<CodecSlot>,
        mut relationships: Vec<DescriptorRelationship>,
        mut placement_groups: Vec<DescriptorPlacementGroup>,
        mut enclaves: Vec<DescriptorEnclave>,
        bounds: DescriptorBounds,
    ) -> Self {
        reactor_slots.sort_by(|left, right| left.id.cmp(&right.id));
        port_slots.sort_by(|left, right| left.id.cmp(&right.id));
        action_slots.sort_by(|left, right| left.id.cmp(&right.id));
        reaction_slots.sort_by(|left, right| left.id.cmp(&right.id));
        mode_slots.sort_by(|left, right| left.id.cmp(&right.id));
        state_slots.sort_by(|left, right| left.id.cmp(&right.id));
        codec_slots.sort_by(|left, right| left.id.cmp(&right.id));
        relationships.sort_by(|left, right| {
            left.reaction
                .cmp(&right.reaction)
                .then_with(|| left.kind.cmp(&right.kind))
                .then_with(|| left.declaration_position.cmp(&right.declaration_position))
                .then_with(|| left.target.cmp(&right.target))
                .then_with(|| left.mode_transition.cmp(&right.mode_transition))
        });
        placement_groups.sort_by(|left, right| left.id.cmp(&right.id));
        enclaves.sort_by(|left, right| left.id.cmp(&right.id));

        let reactor_slots = reactor_slots.into_boxed_slice();
        let port_slots = port_slots.into_boxed_slice();
        let action_slots = action_slots.into_boxed_slice();
        let reaction_slots = reaction_slots.into_boxed_slice();
        let mode_slots = mode_slots.into_boxed_slice();
        let state_slots = state_slots.into_boxed_slice();
        let codec_slots = codec_slots.into_boxed_slice();
        let relationships = relationships.into_boxed_slice();
        let placement_groups = placement_groups.into_boxed_slice();
        let enclaves = enclaves.into_boxed_slice();
        let descriptor_fingerprint_input = DescriptorFingerprintInput {
            contract_id: contract_id.clone(),
            contract_version,
            macro_abi,
            reactor_slots: reactor_slots.clone(),
            port_slots: port_slots.clone(),
            action_slots: action_slots.clone(),
            reaction_slots: reaction_slots.clone(),
            mode_slots: mode_slots.clone(),
            state_slots: state_slots.clone(),
            codec_slots: codec_slots.clone(),
            relationships: relationships.clone(),
            placement_groups: placement_groups.clone(),
            enclaves: enclaves.clone(),
            bounds,
        };

        Self {
            contract_id,
            contract_version,
            macro_abi,
            reactor_slots,
            port_slots,
            action_slots,
            reaction_slots,
            mode_slots,
            state_slots,
            codec_slots,
            relationships,
            placement_groups,
            enclaves,
            bounds,
            descriptor_fingerprint_input,
        }
    }
}
