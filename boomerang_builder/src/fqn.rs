//! Fully-qualified names for elements in the system.

use std::{fmt::Display, ops::Index};

use crate::{
    port::ErasedPortSpec, runtime, ActionSpec, ActionTag, Assembly, AssemblyActionKey,
    AssemblyPortKey, AssemblyReactionKey, AssemblyReactorKey, ParentReactorSpec, PortTag,
    ReactionSpec, ReactorSpec, TypedActionKey, TypedPortKey,
};

use super::AssemblyError;

pub trait FqnSegment {
    /// Create a new segment from a reactor.
    ///
    /// If `grouped` is true, a banked reactor will be represented as a ranged index.
    fn fqn_segment(&self, grouped: bool) -> AssemblyFqnSegment;
}

pub trait Fqn: Copy {
    /// Get a fully-qualified name for self
    ///
    /// If `grouped` is true, the returned Fqn will be grouped by bank
    fn fqn(self, assembly: &Assembly, grouped: bool) -> Result<AssemblyFqn, AssemblyError>;
}

/// The separator for segments in a fully-qualified name.
const FQN_SEGMENT: &str = "/";

/// An index for a segment of a fully-qualified name.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AssemblyFqnSegmentIndex {
    /// The segment is not an array index.
    #[default]
    None,
    /// The segment is an array index.
    Index(usize),
    /// The segment is an array index with a range.
    Range(usize, usize),
}

impl AssemblyFqnSegmentIndex {
    pub fn is_some(&self) -> bool {
        matches!(self, Self::Index(_) | Self::Range(_, _))
    }
}

/// A single segment of a fully-qualified name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AssemblyFqnSegment {
    name: String,
    /// If the segment is an array index, this field will contain the index.
    index: AssemblyFqnSegmentIndex,
}

impl FqnSegment for ReactorSpec {
    /// Create a new segment from a reactor.
    ///
    /// If `grouped` is true, a banked reactor will be represented as a ranged index.
    fn fqn_segment(&self, grouped: bool) -> AssemblyFqnSegment {
        let name = self.name().to_string();
        let index = self
            .bank_info()
            .map(|bi| {
                if grouped {
                    AssemblyFqnSegmentIndex::Range(0, bi.total)
                } else {
                    AssemblyFqnSegmentIndex::Index(bi.idx)
                }
            })
            .unwrap_or_default();
        AssemblyFqnSegment { name, index }
    }
}

impl FqnSegment for ReactionSpec {
    /// Create a new segment from a reaction.
    fn fqn_segment(&self, _grouped: bool) -> AssemblyFqnSegment {
        let name = self.name().unwrap_or("<unnamed_reaction>").to_string();
        AssemblyFqnSegment {
            name,
            index: AssemblyFqnSegmentIndex::None,
        }
    }
}

impl FqnSegment for dyn ErasedPortSpec {
    /// Create a new segment from an action.
    ///
    /// If `grouped` is true, a banked action will be represented as a ranged index.
    fn fqn_segment(&self, grouped: bool) -> AssemblyFqnSegment {
        let index = self
            .bank_info()
            .map(|bi| {
                if grouped {
                    AssemblyFqnSegmentIndex::Range(0, bi.total)
                } else {
                    AssemblyFqnSegmentIndex::Index(bi.idx)
                }
            })
            .unwrap_or_default();
        AssemblyFqnSegment {
            name: self.name().to_string(),
            index,
        }
    }
}

impl FqnSegment for ActionSpec {
    fn fqn_segment(&self, _grouped: bool) -> AssemblyFqnSegment {
        AssemblyFqnSegment {
            name: self.name().to_string(),
            index: AssemblyFqnSegmentIndex::None,
        }
    }
}

impl AssemblyFqnSegment {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn index(&self) -> AssemblyFqnSegmentIndex {
        self.index
    }
}

impl TryFrom<&str> for AssemblyFqnSegment {
    type Error = AssemblyError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        // parse an optional array index from the end of value
        let (name, index) = match value.rfind('[') {
            Some(index_start) => {
                let (name, index) = value.split_at(index_start);
                let index = index.trim_start_matches('[').trim_end_matches(']');
                let index = if let Some(range_sep) = index.find("..") {
                    let (start, end) = index.split_at(range_sep);
                    let start = start
                        .parse()
                        .map_err(|_| AssemblyError::InvalidFqn(value.to_string()))?;
                    let end = end
                        .trim_start_matches("..")
                        .parse()
                        .map_err(|_| AssemblyError::InvalidFqn(value.to_string()))?;
                    AssemblyFqnSegmentIndex::Range(start, end)
                } else {
                    AssemblyFqnSegmentIndex::Index(
                        index
                            .parse()
                            .map_err(|_| AssemblyError::InvalidFqn(value.to_string()))?,
                    )
                };
                (name.to_string(), index)
            }
            None => (value.to_string(), AssemblyFqnSegmentIndex::None),
        };
        // check for empty name
        if name.is_empty() {
            return Err(AssemblyError::InvalidFqn(value.to_string()));
        }
        Ok(Self { name, index })
    }
}

impl Display for AssemblyFqnSegment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.index {
            AssemblyFqnSegmentIndex::None => write!(f, "{}", self.name),
            AssemblyFqnSegmentIndex::Index(index) => write!(f, "{}[{}]", self.name, index),
            AssemblyFqnSegmentIndex::Range(from, to) => {
                write!(f, "{}[{}..{}]", self.name, from, to)
            }
        }
    }
}

/// A fully-qualified name, used to identify a specific element in the system.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AssemblyFqn(Vec<AssemblyFqnSegment>);

impl AssemblyFqn {
    pub fn append(mut self, segment: AssemblyFqnSegment) -> Result<Self, AssemblyError> {
        self.0.push(segment);
        Ok(self)
    }

    pub fn pop(&mut self) -> Option<AssemblyFqnSegment> {
        self.0.pop()
    }

    pub fn peek(&self) -> Option<&AssemblyFqnSegment> {
        self.0.last()
    }

    /// Split the last element from the FQN, returning the new FQN and the last element.
    pub fn split_last(mut self) -> Option<(Self, AssemblyFqnSegment)> {
        self.0.pop().map(|last| (self, last))
    }
}

impl TryFrom<&str> for AssemblyFqn {
    type Error = AssemblyError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let inner = value
            .split(FQN_SEGMENT)
            .map(AssemblyFqnSegment::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        if inner.is_empty() {
            Err(AssemblyError::InvalidFqn(value.to_string()))
        } else {
            Ok(Self(inner))
        }
    }
}

impl FromIterator<AssemblyFqnSegment> for AssemblyFqn {
    fn from_iter<T: IntoIterator<Item = AssemblyFqnSegment>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl std::fmt::Display for AssemblyFqn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, segment) in self.0.iter().enumerate() {
            if i > 0 {
                write!(f, "{FQN_SEGMENT}")?;
            }
            write!(f, "{}", segment)?;
        }
        Ok(())
    }
}

impl Index<usize> for AssemblyFqn {
    type Output = AssemblyFqnSegment;

    fn index(&self, index: usize) -> &Self::Output {
        &self.0[index]
    }
}

impl Fqn for AssemblyReactorKey {
    fn fqn(self, assembly: &Assembly, grouped: bool) -> Result<AssemblyFqn, AssemblyError> {
        let reactor = assembly
            .reactor_specs
            .get(self)
            .ok_or(AssemblyError::ReactorKeyNotFound(self))?;

        let segment = reactor.fqn_segment(grouped);
        if let Some(parent) = reactor.parent_reactor_key() {
            parent.fqn(assembly, grouped)?.append(segment)
        } else {
            Ok(std::iter::once(segment).collect())
        }
    }
}

impl Fqn for AssemblyActionKey {
    fn fqn(self, assembly: &Assembly, grouped: bool) -> Result<AssemblyFqn, AssemblyError> {
        let action = assembly
            .action_specs
            .get(self)
            .ok_or(AssemblyError::ActionKeyNotFound(self))?;
        let segment = action.fqn_segment(grouped);
        action.reactor_key().fqn(assembly, true)?.append(segment)
    }
}

impl<T, Q> Fqn for TypedActionKey<T, Q>
where
    T: runtime::ReactorData,
    Q: ActionTag,
{
    fn fqn(self, assembly: &Assembly, grouped: bool) -> Result<AssemblyFqn, AssemblyError> {
        AssemblyActionKey::from(self).fqn(assembly, grouped)
    }
}

impl Fqn for AssemblyReactionKey {
    fn fqn(self, assembly: &Assembly, grouped: bool) -> Result<AssemblyFqn, AssemblyError> {
        let reaction = assembly
            .reaction_specs
            .get(self)
            .ok_or(AssemblyError::ReactionKeyNotFound(self))?;
        let segment = reaction.fqn_segment(false);
        reaction.reactor_key.fqn(assembly, grouped)?.append(segment)
    }
}

impl Fqn for AssemblyPortKey {
    fn fqn(self, assembly: &Assembly, grouped: bool) -> Result<AssemblyFqn, AssemblyError> {
        let port = assembly
            .port_specs
            .get(self)
            .ok_or(AssemblyError::PortKeyNotFound(self))?;
        let segment = port.fqn_segment(grouped);
        port.get_reactor_key()
            .fqn(assembly, grouped)?
            .append(segment)
    }
}

impl<T, Q, A> Fqn for TypedPortKey<T, Q, A>
where
    T: runtime::ReactorData,
    Q: PortTag,
    A: Copy,
{
    fn fqn(self, assembly: &Assembly, grouped: bool) -> Result<AssemblyFqn, AssemblyError> {
        AssemblyPortKey::from(self).fqn(assembly, grouped)
    }
}

#[cfg(test)]
mod tests {
    use crate::{runtime, Input, PortSpec};

    use super::*;

    #[test]
    fn test_fqn() {
        let fqn = AssemblyFqn::try_from("boomerang/builder/fqn").unwrap();
        assert_eq!(fqn.to_string(), "boomerang/builder/fqn");
        assert_eq!(fqn[0].to_string(), "boomerang");
        assert_eq!(fqn[1].to_string(), "builder");
        assert_eq!(fqn[2].to_string(), "fqn");
    }

    #[test]
    fn test_fqn_segment() {
        let segment = AssemblyFqnSegment::try_from("fqn").unwrap();
        assert_eq!(segment.to_string(), "fqn");
        assert_eq!(segment.index, AssemblyFqnSegmentIndex::None);

        let segment = AssemblyFqnSegment::try_from("fqn[0]").unwrap();
        assert_eq!(segment.to_string(), "fqn[0]");
        assert_eq!(segment.index, AssemblyFqnSegmentIndex::Index(0));

        let segment = AssemblyFqnSegment::try_from("fqn[1..3]").unwrap();
        assert_eq!(segment.to_string(), "fqn[1..3]");
        assert_eq!(segment.index, AssemblyFqnSegmentIndex::Range(1, 3));

        let fqn = AssemblyFqn::try_from("boomerang/fqn[1]/test").unwrap();
        assert_eq!(fqn.to_string(), "boomerang/fqn[1]/test");
        assert_eq!(fqn[0].to_string(), "boomerang");
        assert_eq!(fqn[1].to_string(), "fqn[1]");
        assert_eq!(fqn[1].index, AssemblyFqnSegmentIndex::Index(1));
        assert_eq!(fqn[2].to_string(), "test");

        // test empty segments
        assert!(AssemblyFqnSegment::try_from("").is_err());

        assert!(AssemblyFqn::try_from("boomerang/fqn[1]/").is_err());

        assert_eq!(
            AssemblyFqn::try_from("boomerang/fqn[1]/test").unwrap(),
            AssemblyFqn::try_from("boomerang/fqn[1]/test").unwrap()
        );
    }

    /// Test the FqnSegment trait for ReactorSpec
    #[test]
    fn test_fqn_segment_reactor_builder() {
        let reactor = ReactorSpec::new("TestReactor", "", (), None, None, false);
        let segment = reactor.fqn_segment(false);
        assert_eq!(segment.to_string(), "TestReactor");

        let banked_reactor = ReactorSpec::new(
            "BankedReactor",
            "",
            (),
            None, // Bank info with index 0 and total 10
            Some(runtime::BankInfo { idx: 0, total: 10 }),
            false,
        );
        let segment_ungrouped = banked_reactor.fqn_segment(false);
        assert_eq!(segment_ungrouped.to_string(), "BankedReactor[0]");

        let segment_grouped = banked_reactor.fqn_segment(true);
        assert_eq!(segment_grouped.to_string(), "BankedReactor[0..10]");
    }

    /// Test the FqnSegment trait for the ReactionSpec
    #[test]
    fn test_fqn_segment_reaction_builder() {
        let reaction = ReactionSpec::new(
            Some("TestReaction"),
            AssemblyReactorKey::default(),
            Box::new(|_| runtime::reaction_closure!().into()),
        );
        let segment = reaction.fqn_segment(false);
        assert_eq!(segment.to_string(), "TestReaction");

        // Test that the index is None for reactions
        assert_eq!(segment.index, AssemblyFqnSegmentIndex::None);
    }

    /// Test the FqnSegment trait for ActionSpec
    #[test]
    fn test_fqn_segment_action_builder() {
        let action = ActionSpec::new(
            "TestAction",
            AssemblyReactorKey::default(),
            None,
            crate::ActionType::Shutdown,
        );
        let segment = action.fqn_segment(false);
        assert_eq!(segment.to_string(), "TestAction");

        // Test that the index is None for actions
        assert_eq!(segment.index, AssemblyFqnSegmentIndex::None);
    }

    /// Test the FqnSegment trait for PortSpec
    #[test]
    fn test_fqn_segment_port_builder() {
        let port = PortSpec::<(), Input>::new(
            "TestPort",
            AssemblyReactorKey::default(),
            None, // No bank info
        );
        let segment = (&port as &dyn ErasedPortSpec).fqn_segment(false);
        assert_eq!(segment.to_string(), "TestPort");

        // Test that the index is None for ports without bank info
        assert_eq!(segment.index, AssemblyFqnSegmentIndex::None);

        // Test with bank info
        let port_banked = PortSpec::<(), Input>::new(
            "BankedPort",
            AssemblyReactorKey::default(),
            Some(runtime::BankInfo { idx: 0, total: 10 }),
        );
        let segment_banked = (&port_banked as &dyn ErasedPortSpec).fqn_segment(false);
        assert_eq!(segment_banked.to_string(), "BankedPort[0]");

        let segment_banked_grouped = (&port_banked as &dyn ErasedPortSpec).fqn_segment(true);
        assert_eq!(segment_banked_grouped.to_string(), "BankedPort[0..10]");
    }
}
