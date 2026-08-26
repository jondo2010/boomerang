use crate::compiler::{ActionId, ModeId, PortId, ReactionId, ReactorId};
use std::ops::{BitOr, BitOrAssign};

/// Independent trigger, use, and effect relation bits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReactionRelationFlags(u8);

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
pub enum ReactionRelationTarget {
    /// Stable action target.
    Action(ActionId),
    /// Stable port target.
    Port(PortId),
}

/// One stable reaction dependency relation.
#[derive(Clone, Debug, Eq, PartialEq)]
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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModeTransitionKind {
    /// Reset target mode state on entry.
    Reset,
    /// Restore target mode history on entry.
    History,
}

/// Stable modal transition declaration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModeTransition {
    /// Stable transition target.
    pub target: ModeId,
    /// Transition behavior.
    pub kind: ModeTransitionKind,
}

/// Optional modal metadata used when constructing a reaction.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReactionOptions {
    /// Mode that lexically owns the reaction.
    pub mode: Option<ModeId>,
    /// Modes in which the reaction is enabled.
    pub enabled_modes: Vec<ModeId>,
    /// Modes reset by the reaction.
    pub reset_modes: Vec<ModeId>,
    /// Optional mode transition performed by the reaction.
    pub transition: Option<ModeTransition>,
}

/// Structural reaction declaration using stable identities.
#[derive(Clone, Debug, Eq, PartialEq)]
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
