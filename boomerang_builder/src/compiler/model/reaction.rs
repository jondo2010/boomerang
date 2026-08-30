use crate::compiler::{ActionId, ModeId, PortId, ReactionId, ReactorId};
#[cfg(feature = "host-interchange")]
use serde::{Deserialize, Serialize};
use std::ops::{BitOr, BitOrAssign};

/// Independent trigger, use, and effect relation bits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "host-interchange", derive(Serialize))]
pub struct ReactionRelationFlags(u8);

#[cfg(feature = "host-interchange")]
impl<'de> serde::Deserialize<'de> for ReactionRelationFlags {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let bits = <u8 as serde::Deserialize>::deserialize(deserializer)?;
        if bits == 0 || bits & !0b111 != 0 {
            return Err(serde::de::Error::custom(
                "reaction relation flags must contain only trigger, use, or effect bits",
            ));
        }
        Ok(Self(bits))
    }
}

impl ReactionRelationFlags {
    /// Trigger relation bit.
    pub const TRIGGER: Self = Self(0b001);
    /// Use relation bit.
    pub const USE: Self = Self(0b010);
    /// Effect relation bit.
    pub const EFFECT: Self = Self(0b100);

    /// Reports whether this target triggers the reaction.
    pub fn is_trigger(self) -> bool {
        self.0 & Self::TRIGGER.0 != 0
    }

    /// Reports whether the reaction reads this target.
    pub fn is_use(self) -> bool {
        self.0 & Self::USE.0 != 0
    }

    /// Reports whether the reaction writes or schedules this target.
    pub fn is_effect(self) -> bool {
        self.0 & Self::EFFECT.0 != 0
    }

    pub(super) fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl BitOr for ReactionRelationFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for ReactionRelationFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// Stable action-or-port reaction target.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[cfg_attr(feature = "host-interchange", derive(Deserialize, Serialize))]
pub enum ReactionRelationTarget {
    /// Stable action target.
    Action(ActionId),
    /// Stable port target.
    Port(PortId),
}

/// One stable reaction dependency relation.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "host-interchange", derive(Deserialize, Serialize))]
#[cfg_attr(feature = "host-interchange", serde(deny_unknown_fields))]
pub struct ReactionRelation {
    /// Stable relation target.
    pub(super) target: ReactionRelationTarget,
    /// Independent trigger, use, and effect flags.
    pub(super) flags: ReactionRelationFlags,
    /// Position within the target declaration category.
    pub(super) declaration_position: u32,
}

impl ReactionRelation {
    /// Constructs one stable target relation.
    pub fn new(
        target: ReactionRelationTarget,
        flags: ReactionRelationFlags,
        declaration_position: u32,
    ) -> Self {
        Self {
            target,
            flags,
            declaration_position,
        }
    }

    /// Returns the stable target.
    pub fn target(&self) -> &ReactionRelationTarget {
        &self.target
    }

    /// Returns the independent relation flags.
    pub fn flags(&self) -> ReactionRelationFlags {
        self.flags
    }

    /// Returns the source declaration position.
    pub fn declaration_position(&self) -> u32 {
        self.declaration_position
    }
}

/// Modal transition behavior.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[cfg_attr(feature = "host-interchange", derive(Deserialize, Serialize))]
pub enum ModeTransitionKind {
    /// Reset target mode state on entry.
    Reset,
    /// Restore target mode history on entry.
    History,
}

/// Stable modal transition declaration.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "host-interchange", derive(Deserialize, Serialize))]
#[cfg_attr(feature = "host-interchange", serde(deny_unknown_fields))]
pub struct ModeTransition {
    /// Stable transition target.
    pub target: ModeId,
    /// Transition behavior.
    pub kind: ModeTransitionKind,
}

impl ModeTransition {
    /// Constructs a typed modal transition.
    pub fn new(target: ModeId, kind: ModeTransitionKind) -> Self {
        Self { target, kind }
    }

    /// Returns the stable transition target.
    pub fn target(&self) -> &ModeId {
        &self.target
    }

    /// Returns the transition behavior.
    pub fn kind(&self) -> ModeTransitionKind {
        self.kind
    }
}

/// Optional modal metadata used when constructing a reaction.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
#[cfg_attr(feature = "host-interchange", derive(Deserialize, Serialize))]
#[cfg_attr(feature = "host-interchange", serde(deny_unknown_fields))]
pub struct ReactionOptions {
    /// Reactor-local mode that lexically owns the reaction.
    ///
    /// Inherited scope is expressed by the owning reactor's structural ancestry.
    pub mode: Option<ModeId>,
    /// Reactor-local modes in which the reaction is enabled.
    pub enabled_modes: Vec<ModeId>,
    /// Modes reset by the reaction.
    pub reset_modes: Vec<ModeId>,
    /// Optional mode transition performed by the reaction.
    pub transition: Option<ModeTransition>,
}

impl ReactionOptions {
    /// Returns the optional direct reactor-local mode scope.
    pub fn mode(&self) -> Option<&ModeId> {
        self.mode.as_ref()
    }

    /// Returns the reactor-local enabled-mode set.
    pub fn enabled_modes(&self) -> &[ModeId] {
        &self.enabled_modes
    }

    /// Returns the modes whose reset entry triggers the reaction.
    pub fn reset_modes(&self) -> &[ModeId] {
        &self.reset_modes
    }

    /// Returns the optional typed modal transition.
    pub fn transition(&self) -> Option<&ModeTransition> {
        self.transition.as_ref()
    }
}

/// Structural reaction declaration using stable identities.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "host-interchange", derive(Deserialize, Serialize))]
#[cfg_attr(feature = "host-interchange", serde(deny_unknown_fields))]
pub struct Reaction {
    /// Stable reaction identity.
    pub(super) id: ReactionId,
    /// Owning reactor identity.
    pub(super) reactor: ReactorId,
    /// Stable dependency relations.
    pub(super) relations: Box<[ReactionRelation]>,
    /// Optional modal metadata.
    pub(super) options: ReactionOptions,
}

impl Reaction {
    pub(super) fn canonicalize(&mut self) {
        self.relations.sort_by(|left, right| {
            let left_category = u8::from(matches!(left.target, ReactionRelationTarget::Port(_)));
            let right_category = u8::from(matches!(right.target, ReactionRelationTarget::Port(_)));
            left_category
                .cmp(&right_category)
                .then_with(|| left.declaration_position.cmp(&right.declaration_position))
                .then_with(|| match (&left.target, &right.target) {
                    (
                        ReactionRelationTarget::Action(left),
                        ReactionRelationTarget::Action(right),
                    ) => left.cmp(right),
                    (ReactionRelationTarget::Port(left), ReactionRelationTarget::Port(right)) => {
                        left.cmp(right)
                    }
                    _ => std::cmp::Ordering::Equal,
                })
        });
        self.options.enabled_modes.sort();
        self.options.reset_modes.sort();
    }

    /// Returns the stable reaction identity.
    pub fn id(&self) -> &ReactionId {
        &self.id
    }

    /// Returns the owning reactor identity.
    pub fn reactor(&self) -> &ReactorId {
        &self.reactor
    }

    /// Returns the dependency relations.
    pub fn relations(&self) -> &[ReactionRelation] {
        &self.relations
    }

    /// Returns the optional modal metadata.
    pub fn options(&self) -> &ReactionOptions {
        &self.options
    }

    /// Returns the number of dependency relations.
    pub fn relation_count(&self) -> usize {
        self.relations.len()
    }
}
