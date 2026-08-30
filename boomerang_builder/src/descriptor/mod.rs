//! Canonical, validated component descriptor records owned by the host-side builder.

use crate::compiler::{BindingSlotId, ContractId, ModeTransitionKind, PortDirection, StablePath};
#[cfg(feature = "host-interchange")]
use serde::{Deserialize, Serialize};

#[cfg(feature = "descriptor-fingerprint")]
mod fingerprint;

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
/// Marker for stable placement-group binding slots.
pub enum PlacementGroupSlotMarker {}
/// Marker for stable Enclave binding slots.
pub enum EnclaveSlotMarker {}

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
/// Stable implementation-local placement-group slot identity.
pub type PlacementGroupSlotId = BindingSlotId<PlacementGroupSlotMarker>;
/// Stable implementation-local Enclave slot identity.
pub type EnclaveSlotId = BindingSlotId<EnclaveSlotMarker>;

/// Stable reactor slot and its structural parent.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "host-interchange", derive(Deserialize, Serialize))]
#[cfg_attr(feature = "host-interchange", serde(deny_unknown_fields))]
pub struct ReactorSlot {
    /// Stable implementation-local identity.
    pub id: ReactorSlotId,
    /// Optional parent reactor slot.
    pub parent: Option<ReactorSlotId>,
}

/// Stable port slot owned by one reactor slot.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "host-interchange", derive(Deserialize, Serialize))]
#[cfg_attr(feature = "host-interchange", serde(deny_unknown_fields))]
pub struct PortSlot {
    /// Stable implementation-local identity.
    pub id: PortSlotId,
    /// Owning reactor slot.
    pub reactor: ReactorSlotId,
    /// Structural port direction.
    pub direction: PortDirection,
}

/// Stable action slot owned by one reactor slot.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "host-interchange", derive(Deserialize, Serialize))]
#[cfg_attr(feature = "host-interchange", serde(deny_unknown_fields))]
pub struct ActionSlot {
    /// Stable implementation-local identity.
    pub id: ActionSlotId,
    /// Owning reactor slot.
    pub reactor: ReactorSlotId,
}

/// Stable reaction slot owned by one reactor slot.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "host-interchange", derive(Deserialize, Serialize))]
#[cfg_attr(feature = "host-interchange", serde(deny_unknown_fields))]
pub struct ReactionSlot {
    /// Stable implementation-local identity.
    pub id: ReactionSlotId,
    /// Owning reactor slot.
    pub reactor: ReactorSlotId,
}

/// Stable mode slot owned by one reactor slot.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "host-interchange", derive(Deserialize, Serialize))]
#[cfg_attr(feature = "host-interchange", serde(deny_unknown_fields))]
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
#[cfg_attr(feature = "host-interchange", derive(Deserialize, Serialize))]
#[cfg_attr(feature = "host-interchange", serde(deny_unknown_fields))]
pub struct StateSlot {
    /// Stable implementation-local identity.
    pub id: StateSlotId,
    /// Owning reactor slot.
    pub reactor: ReactorSlotId,
}

/// Stable payload codec binding slot.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "host-interchange", derive(Deserialize, Serialize))]
#[cfg_attr(feature = "host-interchange", serde(deny_unknown_fields))]
pub struct CodecSlot {
    /// Stable implementation-local identity.
    pub id: CodecSlotId,
}

/// Source-observable lifecycle trigger.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[cfg_attr(feature = "host-interchange", derive(Deserialize, Serialize))]
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
#[cfg_attr(feature = "host-interchange", derive(Deserialize, Serialize))]
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
#[cfg_attr(feature = "host-interchange", derive(Deserialize, Serialize))]
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

/// One canonical relationship from a reaction to a stable target.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "host-interchange", derive(Deserialize, Serialize))]
#[cfg_attr(feature = "host-interchange", serde(deny_unknown_fields))]
pub struct DescriptorRelationship {
    /// Source reaction slot.
    pub reaction: ReactionSlotId,
    /// Relationship role.
    pub kind: DescriptorRelationshipKind,
    /// Stable relationship target.
    pub target: DescriptorRelationshipTarget,
    /// Optional modal transition semantics.
    pub mode_transition: Option<ModeTransitionKind>,
    /// Zero-based position within this relationship category.
    pub declaration_position: u32,
}

/// Source-declared placement group visible to deployment selection.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "host-interchange", derive(Deserialize, Serialize))]
#[cfg_attr(feature = "host-interchange", serde(deny_unknown_fields))]
pub struct DescriptorPlacementGroup {
    /// Stable placement-group identity.
    pub id: PlacementGroupSlotId,
    /// Optional parent placement group.
    pub parent: Option<PlacementGroupSlotId>,
}

/// Source-declared scheduler and logical-time domain.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "host-interchange", derive(Deserialize, Serialize))]
#[cfg_attr(feature = "host-interchange", serde(deny_unknown_fields))]
pub struct DescriptorEnclave {
    /// Stable Enclave identity.
    pub id: EnclaveSlotId,
    /// Stable root reactor slot.
    pub root: ReactorSlotId,
}

/// A deployability bound that may not yet be expressed by source syntax.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "host-interchange", derive(Deserialize, Serialize))]
pub enum DescriptorBound {
    /// Source syntax did not declare this bound.
    Unknown,
    /// Declared inclusive upper bound.
    Known(u64),
}

/// Declared storage and queue bounds for one component implementation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "host-interchange", derive(Deserialize, Serialize))]
#[cfg_attr(feature = "host-interchange", serde(deny_unknown_fields))]
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

/// Canonically ordered, validated typed input to the descriptor fingerprint encoder.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "host-interchange", derive(Serialize))]
pub struct DescriptorFingerprintInput {
    /// Stable external contract identity.
    contract_id: ContractId,
    /// Stable external contract version.
    contract_version: u64,
    /// Descriptor macro ABI version.
    macro_abi: u32,
    /// Canonically ordered reactor slots.
    reactor_slots: Box<[ReactorSlot]>,
    /// Canonically ordered port slots.
    port_slots: Box<[PortSlot]>,
    /// Canonically ordered action slots.
    action_slots: Box<[ActionSlot]>,
    /// Canonically ordered reaction slots.
    reaction_slots: Box<[ReactionSlot]>,
    /// Canonically ordered mode slots.
    mode_slots: Box<[ModeSlot]>,
    /// Canonically ordered state slots.
    state_slots: Box<[StateSlot]>,
    /// Canonically ordered codec slots.
    codec_slots: Box<[CodecSlot]>,
    /// Canonically ordered reaction relationships.
    relationships: Box<[DescriptorRelationship]>,
    /// Canonically ordered placement groups.
    placement_groups: Box<[DescriptorPlacementGroup]>,
    /// Canonically ordered Enclaves.
    enclaves: Box<[DescriptorEnclave]>,
    /// Declared resource bounds.
    bounds: DescriptorBounds,
}

macro_rules! descriptor_accessors {
    ($($field:ident)?) => {
        /// Returns the stable external contract identity.
        pub fn contract_id(&self) -> &ContractId {
            &self$(.$field)?.contract_id
        }
        /// Returns the stable external contract version.
        pub fn contract_version(&self) -> u64 {
            self$(.$field)?.contract_version
        }
        /// Returns the descriptor macro ABI version.
        pub fn macro_abi(&self) -> u32 {
            self$(.$field)?.macro_abi
        }
        /// Returns the canonically ordered reactor slots.
        pub fn reactor_slots(&self) -> &[ReactorSlot] {
            &self$(.$field)?.reactor_slots
        }
        /// Returns the canonically ordered port slots.
        pub fn port_slots(&self) -> &[PortSlot] {
            &self$(.$field)?.port_slots
        }
        /// Returns the canonically ordered action slots.
        pub fn action_slots(&self) -> &[ActionSlot] {
            &self$(.$field)?.action_slots
        }
        /// Returns the canonically ordered reaction slots.
        pub fn reaction_slots(&self) -> &[ReactionSlot] {
            &self$(.$field)?.reaction_slots
        }
        /// Returns the canonically ordered mode slots.
        pub fn mode_slots(&self) -> &[ModeSlot] {
            &self$(.$field)?.mode_slots
        }
        /// Returns the canonically ordered state slots.
        pub fn state_slots(&self) -> &[StateSlot] {
            &self$(.$field)?.state_slots
        }
        /// Returns the canonically ordered codec slots.
        pub fn codec_slots(&self) -> &[CodecSlot] {
            &self$(.$field)?.codec_slots
        }
        /// Returns the canonically ordered reaction relationships.
        pub fn relationships(&self) -> &[DescriptorRelationship] {
            &self$(.$field)?.relationships
        }
        /// Returns the canonically ordered placement groups.
        pub fn placement_groups(&self) -> &[DescriptorPlacementGroup] {
            &self$(.$field)?.placement_groups
        }
        /// Returns the canonically ordered Enclaves.
        pub fn enclaves(&self) -> &[DescriptorEnclave] {
            &self$(.$field)?.enclaves
        }
        /// Returns the declared resource bounds.
        pub fn bounds(&self) -> DescriptorBounds {
            self$(.$field)?.bounds
        }
    };
}

impl DescriptorFingerprintInput {
    descriptor_accessors!();
}

/// Invalid structure supplied while constructing a component descriptor.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum DescriptorBuildError {
    /// Two slots in the same binding category have the same identity.
    #[error("duplicate {kind} slot `{id}`")]
    DuplicateSlot {
        /// Binding category containing the duplicate.
        kind: &'static str,
        /// Duplicated stable identity.
        id: String,
    },
    /// A relationship refers to a slot absent from the descriptor.
    #[error("relationship from `{reaction}` references missing {target}")]
    DanglingRelation {
        /// Source reaction identity.
        reaction: String,
        /// Missing source or target identity and category.
        target: String,
    },
    /// A structural record refers to an absent typed owner or parent.
    #[error("{owner_kind} `{owner}` references missing {target_kind} `{target}`")]
    DanglingReference {
        /// Kind of record containing the reference.
        owner_kind: &'static str,
        /// Stable identity of that record.
        owner: String,
        /// Kind of referenced record.
        target_kind: &'static str,
        /// Missing stable identity.
        target: String,
    },
}

/// Host-owned structural descriptor generated for one component implementation.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "host-interchange", derive(Serialize))]
pub struct ComponentDescriptor {
    /// Single canonical descriptor record and fingerprint input.
    canonical: DescriptorFingerprintInput,
}

#[cfg(feature = "host-interchange")]
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct UncheckedDescriptor {
    contract_id: ContractId,
    contract_version: u64,
    macro_abi: u32,
    reactor_slots: Vec<ReactorSlot>,
    port_slots: Vec<PortSlot>,
    action_slots: Vec<ActionSlot>,
    reaction_slots: Vec<ReactionSlot>,
    mode_slots: Vec<ModeSlot>,
    state_slots: Vec<StateSlot>,
    codec_slots: Vec<CodecSlot>,
    relationships: Vec<DescriptorRelationship>,
    placement_groups: Vec<DescriptorPlacementGroup>,
    enclaves: Vec<DescriptorEnclave>,
    bounds: DescriptorBounds,
}

#[cfg(feature = "host-interchange")]
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct UncheckedComponentDescriptor {
    canonical: UncheckedDescriptor,
}

#[cfg(feature = "host-interchange")]
impl<'de> serde::Deserialize<'de> for ComponentDescriptor {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;

        let unchecked = UncheckedComponentDescriptor::deserialize(deserializer)?.canonical;
        Self::try_new(
            unchecked.contract_id,
            unchecked.contract_version,
            unchecked.macro_abi,
            unchecked.reactor_slots,
            unchecked.port_slots,
            unchecked.action_slots,
            unchecked.reaction_slots,
            unchecked.mode_slots,
            unchecked.state_slots,
            unchecked.codec_slots,
            unchecked.relationships,
            unchecked.placement_groups,
            unchecked.enclaves,
            unchecked.bounds,
        )
        .map_err(D::Error::custom)
    }
}

impl ComponentDescriptor {
    descriptor_accessors!(canonical);

    /// Returns the immutable canonical fingerprint input.
    pub fn descriptor_fingerprint_input(&self) -> &DescriptorFingerprintInput {
        &self.canonical
    }

    /// Validates, constructs, and canonically orders one descriptor.
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
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
    ) -> Result<Self, DescriptorBuildError> {
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

        reject_duplicate_slots(&reactor_slots, "reactor", |slot| &slot.id)?;
        reject_duplicate_slots(&port_slots, "port", |slot| &slot.id)?;
        reject_duplicate_slots(&action_slots, "action", |slot| &slot.id)?;
        reject_duplicate_slots(&reaction_slots, "reaction", |slot| &slot.id)?;
        reject_duplicate_slots(&mode_slots, "mode", |slot| &slot.id)?;
        reject_duplicate_slots(&state_slots, "state", |slot| &slot.id)?;
        reject_duplicate_slots(&codec_slots, "codec", |slot| &slot.id)?;
        reject_duplicate_slots(&placement_groups, "placement group", |slot| &slot.id)?;
        reject_duplicate_slots(&enclaves, "Enclave", |slot| &slot.id)?;

        let missing = |owner_kind, owner: String, target_kind, target: String| {
            DescriptorBuildError::DanglingReference {
                owner_kind,
                owner,
                target_kind,
                target,
            }
        };
        let has_reactor = |id: &ReactorSlotId| {
            reactor_slots
                .binary_search_by(|slot| slot.id.cmp(id))
                .is_ok()
        };
        for slot in &reactor_slots {
            if let Some(parent) = &slot.parent {
                if !has_reactor(parent) {
                    return Err(missing(
                        "reactor",
                        slot.id.to_string(),
                        "reactor",
                        parent.to_string(),
                    ));
                }
            }
        }
        macro_rules! validate_reactor_owners {
            ($slots:expr, $kind:literal) => {
                for slot in $slots {
                    if !has_reactor(&slot.reactor) {
                        return Err(missing(
                            $kind,
                            slot.id.to_string(),
                            "reactor",
                            slot.reactor.to_string(),
                        ));
                    }
                }
            };
        }
        validate_reactor_owners!(&port_slots, "port");
        validate_reactor_owners!(&action_slots, "action");
        validate_reactor_owners!(&reaction_slots, "reaction");
        validate_reactor_owners!(&mode_slots, "mode");
        validate_reactor_owners!(&state_slots, "state");
        for slot in &mode_slots {
            if let Some(parent) = &slot.parent {
                if mode_slots
                    .binary_search_by(|candidate| candidate.id.cmp(parent))
                    .is_err()
                {
                    return Err(missing(
                        "mode",
                        slot.id.to_string(),
                        "mode",
                        parent.to_string(),
                    ));
                }
            }
        }
        for group in &placement_groups {
            if let Some(parent) = &group.parent {
                if placement_groups
                    .binary_search_by(|candidate| candidate.id.cmp(parent))
                    .is_err()
                {
                    return Err(missing(
                        "placement group",
                        group.id.to_string(),
                        "placement group",
                        parent.to_string(),
                    ));
                }
            }
        }
        for enclave in &enclaves {
            if !has_reactor(&enclave.root) {
                return Err(missing(
                    "Enclave",
                    enclave.id.to_string(),
                    "reactor",
                    enclave.root.to_string(),
                ));
            }
        }

        for relationship in &relationships {
            let reaction = relationship.reaction.to_string();
            if reaction_slots
                .binary_search_by(|slot| slot.id.cmp(&relationship.reaction))
                .is_err()
            {
                return Err(DescriptorBuildError::DanglingRelation {
                    target: format!("reaction slot `{reaction}`"),
                    reaction,
                });
            }
            let target = match &relationship.target {
                DescriptorRelationshipTarget::Port(id)
                    if port_slots.binary_search_by(|slot| slot.id.cmp(id)).is_err() =>
                {
                    Some(format!("port slot `{id}`"))
                }
                DescriptorRelationshipTarget::Action(id)
                    if action_slots
                        .binary_search_by(|slot| slot.id.cmp(id))
                        .is_err() =>
                {
                    Some(format!("action slot `{id}`"))
                }
                DescriptorRelationshipTarget::Mode(id)
                    if mode_slots.binary_search_by(|slot| slot.id.cmp(id)).is_err() =>
                {
                    Some(format!("mode slot `{id}`"))
                }
                _ => None,
            };
            if let Some(target) = target {
                return Err(DescriptorBuildError::DanglingRelation { reaction, target });
            }
        }

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
        let canonical = DescriptorFingerprintInput {
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
        };

        Ok(Self { canonical })
    }

    /// Constructs a descriptor whose literals and structure were validated by the reactor macro.
    #[doc(hidden)]
    #[allow(clippy::too_many_arguments)]
    pub fn __from_macro(
        contract: &'static str,
        contract_version: u64,
        macro_abi: u32,
        reactor_slots: Vec<ReactorSlot>,
        port_slots: Vec<PortSlot>,
        action_slots: Vec<ActionSlot>,
        reaction_slots: Vec<ReactionSlot>,
        mode_slots: Vec<ModeSlot>,
        state_slots: Vec<StateSlot>,
        codec_slots: Vec<CodecSlot>,
        relationships: Vec<DescriptorRelationship>,
        placement_groups: Vec<DescriptorPlacementGroup>,
        enclaves: Vec<DescriptorEnclave>,
        bounds: DescriptorBounds,
    ) -> Self {
        let contract_id = ContractId::new(contract).expect("reactor macro validated contract text");
        Self::try_new(
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
        )
        .expect("reactor macro generated a valid component descriptor")
    }
}

fn reject_duplicate_slots<T, I>(
    slots: &[T],
    kind: &'static str,
    id: impl Fn(&T) -> &I,
) -> Result<(), DescriptorBuildError>
where
    I: Eq + ToString,
{
    if let Some(pair) = slots.windows(2).find(|pair| id(&pair[0]) == id(&pair[1])) {
        return Err(DescriptorBuildError::DuplicateSlot {
            kind,
            id: id(&pair[0]).to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Parts {
        reactors: Vec<ReactorSlot>,
        ports: Vec<PortSlot>,
        actions: Vec<ActionSlot>,
        reactions: Vec<ReactionSlot>,
        modes: Vec<ModeSlot>,
        states: Vec<StateSlot>,
        /// Payload codecs supplied to the descriptor builder.
        codecs: Vec<CodecSlot>,
        relationships: Vec<DescriptorRelationship>,
        groups: Vec<DescriptorPlacementGroup>,
        enclaves: Vec<DescriptorEnclave>,
    }

    fn reactor() -> ReactorSlot {
        ReactorSlot {
            id: ReactorSlotId::new("Root").unwrap(),
            parent: None,
        }
    }

    fn port(name: &str) -> PortSlot {
        PortSlot {
            id: PortSlotId::new(format!("Root/{name}")).unwrap(),
            reactor: ReactorSlotId::new("Root").unwrap(),
            direction: PortDirection::Input,
        }
    }

    fn build(
        ports: Vec<PortSlot>,
        reactions: Vec<ReactionSlot>,
        relationships: Vec<DescriptorRelationship>,
    ) -> Result<ComponentDescriptor, DescriptorBuildError> {
        build_parts(Parts {
            reactors: vec![reactor()],
            ports,
            reactions,
            relationships,
            ..Parts::default()
        })
    }

    fn build_parts(parts: Parts) -> Result<ComponentDescriptor, DescriptorBuildError> {
        ComponentDescriptor::try_new(
            ContractId::new("example.contract").unwrap(),
            1,
            COMPONENT_DESCRIPTOR_MACRO_ABI,
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
            DescriptorBounds::default(),
        )
    }

    #[test]
    fn canonical_types_and_local_deployment_slots_are_preserved() {
        let root = ReactorSlotId::new("Root").unwrap();
        let relation = DescriptorRelationship {
            reaction: ReactionSlotId::new("Root/r").unwrap(),
            kind: DescriptorRelationshipKind::Mode,
            target: DescriptorRelationshipTarget::Mode(ModeSlotId::new("Root/m").unwrap()),
            mode_transition: Some(ModeTransitionKind::Reset),
            declaration_position: 0,
        };
        let descriptor = build_parts(Parts {
            reactors: vec![reactor()],
            reactions: vec![ReactionSlot {
                id: relation.reaction.clone(),
                reactor: root.clone(),
            }],
            modes: vec![ModeSlot {
                id: ModeSlotId::new("Root/m").unwrap(),
                reactor: root.clone(),
                parent: None,
                initial: true,
            }],
            relationships: vec![relation],
            groups: vec![
                DescriptorPlacementGroup {
                    id: PlacementGroupSlotId::new("z").unwrap(),
                    parent: None,
                },
                DescriptorPlacementGroup {
                    id: PlacementGroupSlotId::new("a").unwrap(),
                    parent: None,
                },
            ],
            enclaves: vec![
                DescriptorEnclave {
                    id: EnclaveSlotId::new("z").unwrap(),
                    root: root.clone(),
                },
                DescriptorEnclave {
                    id: EnclaveSlotId::new("a").unwrap(),
                    root,
                },
            ],
            ..Parts::default()
        })
        .unwrap();
        assert_eq!(
            descriptor.port_slots().first().map(|slot| slot.direction),
            None
        );
        assert_eq!(
            descriptor.relationships()[0].mode_transition,
            Some(ModeTransitionKind::Reset)
        );
        assert_eq!(descriptor.placement_groups()[0].id.to_string(), "a");
        assert_eq!(descriptor.enclaves()[0].id.to_string(), "a");
    }

    #[test]
    fn duplicate_slots_and_dangling_relations_are_rejected() {
        assert!(matches!(
            build(vec![port("a"), port("a")], vec![], vec![]),
            Err(DescriptorBuildError::DuplicateSlot { kind: "port", .. })
        ));

        let missing_reaction = ReactionSlotId::new("Root/missing").unwrap();
        let relation = DescriptorRelationship {
            reaction: missing_reaction,
            kind: DescriptorRelationshipKind::Trigger,
            target: DescriptorRelationshipTarget::Lifecycle(DescriptorLifecycle::Startup),
            mode_transition: None,
            declaration_position: 0,
        };
        assert!(matches!(
            build(vec![], vec![], vec![relation]),
            Err(DescriptorBuildError::DanglingRelation { .. })
        ));

        let reaction = ReactionSlot {
            id: ReactionSlotId::new("Root/reaction").unwrap(),
            reactor: ReactorSlotId::new("Root").unwrap(),
        };
        let relation = DescriptorRelationship {
            reaction: reaction.id.clone(),
            kind: DescriptorRelationshipKind::Use,
            target: DescriptorRelationshipTarget::Port(PortSlotId::new("Root/missing").unwrap()),
            mode_transition: None,
            declaration_position: 0,
        };
        assert!(matches!(
            build(vec![], vec![reaction], vec![relation]),
            Err(DescriptorBuildError::DanglingRelation { .. })
        ));
    }

    #[test]
    fn dangling_structural_references_are_rejected() {
        let missing_reactor = ReactorSlotId::new("Missing").unwrap();
        macro_rules! dangling {
            ($kind:literal, $field:ident: $value:expr) => {
                let mut parts = Parts {
                    reactors: vec![reactor()],
                    ..Parts::default()
                };
                parts.$field = $value;
                assert!(matches!(
                    build_parts(parts),
                    Err(DescriptorBuildError::DanglingReference {
                        owner_kind: $kind,
                        ..
                    })
                ));
            };
        }
        dangling!("reactor", reactors: vec![ReactorSlot { id: ReactorSlotId::new("Root").unwrap(), parent: Some(missing_reactor.clone()) }]);
        dangling!("port", ports: vec![PortSlot { id: PortSlotId::new("Root/p").unwrap(), reactor: missing_reactor.clone(), direction: PortDirection::Input }]);
        dangling!("action", actions: vec![ActionSlot { id: ActionSlotId::new("Root/a").unwrap(), reactor: missing_reactor.clone() }]);
        dangling!("reaction", reactions: vec![ReactionSlot { id: ReactionSlotId::new("Root/r").unwrap(), reactor: missing_reactor.clone() }]);
        dangling!("mode", modes: vec![ModeSlot { id: ModeSlotId::new("Root/m").unwrap(), reactor: missing_reactor.clone(), parent: None, initial: false }]);
        dangling!("state", states: vec![StateSlot { id: StateSlotId::new("Root/s").unwrap(), reactor: missing_reactor.clone() }]);
        dangling!("mode", modes: vec![ModeSlot { id: ModeSlotId::new("Root/m").unwrap(), reactor: ReactorSlotId::new("Root").unwrap(), parent: Some(ModeSlotId::new("Root/missing").unwrap()), initial: false }]);
        dangling!("placement group", groups: vec![DescriptorPlacementGroup { id: PlacementGroupSlotId::new("group").unwrap(), parent: Some(PlacementGroupSlotId::new("missing").unwrap()) }]);
        dangling!("Enclave", enclaves: vec![DescriptorEnclave { id: EnclaveSlotId::new("enclave").unwrap(), root: missing_reactor }]);
    }
}
