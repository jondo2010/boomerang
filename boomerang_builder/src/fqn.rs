//! Fully-qualified names for elements in the system.

use std::{fmt::Display, ops::Index};

use crate::{
    runtime, ActionBuilder, BasePortBuilder, BuilderActionKey, BuilderPortKey, BuilderReactionKey,
    BuilderReactorKey, EnvBuilder, ParentReactorBuilder, ReactionBuilder, ReactorBuilder,
};

use super::BuilderError;

pub trait FqnSegment {
    /// Create a new segment from a reactor.
    ///
    /// If `grouped` is true, a banked reactor will be represented as a ranged index.
    fn fqn_segment(&self, grouped: bool) -> BuilderFqnSegment;
}

pub trait Fqn: Copy {
    /// Get a fully-qualified name for self
    ///
    /// If `grouped` is true, the returned Fqn will be grouped by bank
    fn fqn(self, env: &EnvBuilder, grouped: bool) -> Result<BuilderFqn, BuilderError>;
}

/// The separator for segments in a fully-qualified name.
const FQN_SEGMENT: &str = "/";

/// An index for a segment of a fully-qualified name.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BuilderFqnSegmentIndex {
    /// The segment is not an array index.
    #[default]
    None,
    /// The segment is an array index.
    Index(usize),
    /// The segment is an array index with a range.
    Range(usize, usize),
}

impl BuilderFqnSegmentIndex {
    pub fn is_some(&self) -> bool {
        matches!(self, Self::Index(_) | Self::Range(_, _))
    }
}

/// A single segment of a fully-qualified name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct BuilderFqnSegment {
    name: String,
    /// If the segment is an array index, this field will contain the index.
    index: BuilderFqnSegmentIndex,
}

impl FqnSegment for ReactorBuilder {
    /// Create a new segment from a reactor.
    ///
    /// If `grouped` is true, a banked reactor will be represented as a ranged index.
    fn fqn_segment(&self, grouped: bool) -> BuilderFqnSegment {
        let name = self.name().to_string();
        let index = self
            .bank_info()
            .map(|bi| {
                if grouped {
                    BuilderFqnSegmentIndex::Range(0, bi.total)
                } else {
                    BuilderFqnSegmentIndex::Index(bi.idx)
                }
            })
            .unwrap_or_default();
        BuilderFqnSegment { name, index }
    }
}

impl FqnSegment for ReactionBuilder {
    /// Create a new segment from a reaction.
    fn fqn_segment(&self, _grouped: bool) -> BuilderFqnSegment {
        let name = self.name().to_string();
        BuilderFqnSegment {
            name,
            index: BuilderFqnSegmentIndex::None,
        }
    }
}

impl FqnSegment for dyn BasePortBuilder {
    /// Create a new segment from an action.
    ///
    /// If `grouped` is true, a banked action will be represented as a ranged index.
    fn fqn_segment(&self, grouped: bool) -> BuilderFqnSegment {
        let index = self
            .bank_info()
            .map(|bi| {
                if grouped {
                    BuilderFqnSegmentIndex::Range(0, bi.total)
                } else {
                    BuilderFqnSegmentIndex::Index(bi.idx)
                }
            })
            .unwrap_or_default();
        BuilderFqnSegment {
            name: self.name().to_string(),
            index,
        }
    }
}

impl FqnSegment for ActionBuilder {
    fn fqn_segment(&self, _grouped: bool) -> BuilderFqnSegment {
        BuilderFqnSegment {
            name: self.name().to_string(),
            index: BuilderFqnSegmentIndex::None,
        }
    }
}

impl BuilderFqnSegment {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn index(&self) -> BuilderFqnSegmentIndex {
        self.index
    }
}

impl TryFrom<&str> for BuilderFqnSegment {
    type Error = BuilderError;

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
                        .map_err(|_| BuilderError::InvalidFqn(value.to_string()))?;
                    let end = end
                        .trim_start_matches("..")
                        .parse()
                        .map_err(|_| BuilderError::InvalidFqn(value.to_string()))?;
                    BuilderFqnSegmentIndex::Range(start, end)
                } else {
                    BuilderFqnSegmentIndex::Index(
                        index
                            .parse()
                            .map_err(|_| BuilderError::InvalidFqn(value.to_string()))?,
                    )
                };
                (name.to_string(), index)
            }
            None => (value.to_string(), BuilderFqnSegmentIndex::None),
        };
        // check for empty name
        if name.is_empty() {
            return Err(BuilderError::InvalidFqn(value.to_string()));
        }
        Ok(Self { name, index })
    }
}

impl Display for BuilderFqnSegment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.index {
            BuilderFqnSegmentIndex::None => write!(f, "{}", self.name),
            BuilderFqnSegmentIndex::Index(index) => write!(f, "{}[{}]", self.name, index),
            BuilderFqnSegmentIndex::Range(from, to) => {
                write!(f, "{}[{}..{}]", self.name, from, to)
            }
        }
    }
}

/// A fully-qualified name, used to identify a specific element in the system.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct BuilderFqn(Vec<BuilderFqnSegment>);

impl BuilderFqn {
    pub fn append(mut self, segment: BuilderFqnSegment) -> Result<Self, BuilderError> {
        self.0.push(segment);
        Ok(self)
    }

    pub fn pop(&mut self) -> Option<BuilderFqnSegment> {
        self.0.pop()
    }

    pub fn peek(&self) -> Option<&BuilderFqnSegment> {
        self.0.last()
    }

    /// Split the last element from the FQN, returning the new FQN and the last element.
    pub fn split_last(mut self) -> Option<(Self, BuilderFqnSegment)> {
        self.0.pop().map(|last| (self, last))
    }
}

impl TryFrom<&str> for BuilderFqn {
    type Error = BuilderError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let inner = value
            .split(FQN_SEGMENT)
            .map(BuilderFqnSegment::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        if inner.is_empty() {
            Err(BuilderError::InvalidFqn(value.to_string()))
        } else {
            Ok(Self(inner))
        }
    }
}

impl FromIterator<BuilderFqnSegment> for BuilderFqn {
    fn from_iter<T: IntoIterator<Item = BuilderFqnSegment>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl std::fmt::Display for BuilderFqn {
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

impl Index<usize> for BuilderFqn {
    type Output = BuilderFqnSegment;

    fn index(&self, index: usize) -> &Self::Output {
        &self.0[index]
    }
}

impl Fqn for BuilderReactorKey {
    fn fqn(self, env: &EnvBuilder, grouped: bool) -> Result<BuilderFqn, BuilderError> {
        let reactor = env
            .reactor_builders
            .get(self)
            .ok_or(BuilderError::ReactorKeyNotFound(self))?;

        let segment = reactor.fqn_segment(grouped);
        if let Some(parent) = reactor.parent_reactor_key() {
            parent.fqn(env, grouped)?.append(segment)
        } else {
            Ok(std::iter::once(segment).collect())
        }
    }
}

impl Fqn for BuilderActionKey {
    fn fqn(self, env: &EnvBuilder, grouped: bool) -> Result<BuilderFqn, BuilderError> {
        let action = env
            .action_builders
            .get(self)
            .ok_or(BuilderError::ActionKeyNotFound(self))?;
        let segment = action.fqn_segment(grouped);
        action.reactor_key().fqn(env, true)?.append(segment)
    }
}

impl Fqn for BuilderReactionKey {
    fn fqn(self, env: &EnvBuilder, grouped: bool) -> Result<BuilderFqn, BuilderError> {
        let reaction = env
            .reaction_builders
            .get(self)
            .ok_or(BuilderError::ReactionKeyNotFound(self))?;
        let segment = reaction.fqn_segment(false);
        reaction.reactor_key.fqn(env, grouped)?.append(segment)
    }
}

impl Fqn for BuilderPortKey {
    fn fqn(self, env: &EnvBuilder, grouped: bool) -> Result<BuilderFqn, BuilderError> {
        let port = env
            .port_builders
            .get(self)
            .ok_or(BuilderError::PortKeyNotFound(self))?;
        let segment = port.fqn_segment(grouped);
        port.get_reactor_key().fqn(env, grouped)?.append(segment)
    }
}

#[cfg(test)]
mod tests {
    use crate::{Input, PortBuilder};

    use super::*;

    #[test]
    fn test_fqn() {
        let fqn = BuilderFqn::try_from("boomerang/builder/fqn").unwrap();
        assert_eq!(fqn.to_string(), "boomerang/builder/fqn");
        assert_eq!(fqn[0].to_string(), "boomerang");
        assert_eq!(fqn[1].to_string(), "builder");
        assert_eq!(fqn[2].to_string(), "fqn");
    }

    #[test]
    fn test_fqn_segment() {
        let segment = BuilderFqnSegment::try_from("fqn").unwrap();
        assert_eq!(segment.to_string(), "fqn");
        assert_eq!(segment.index, BuilderFqnSegmentIndex::None);

        let segment = BuilderFqnSegment::try_from("fqn[0]").unwrap();
        assert_eq!(segment.to_string(), "fqn[0]");
        assert_eq!(segment.index, BuilderFqnSegmentIndex::Index(0));

        let segment = BuilderFqnSegment::try_from("fqn[1..3]").unwrap();
        assert_eq!(segment.to_string(), "fqn[1..3]");
        assert_eq!(segment.index, BuilderFqnSegmentIndex::Range(1, 3));

        let fqn = BuilderFqn::try_from("boomerang/fqn[1]/test").unwrap();
        assert_eq!(fqn.to_string(), "boomerang/fqn[1]/test");
        assert_eq!(fqn[0].to_string(), "boomerang");
        assert_eq!(fqn[1].to_string(), "fqn[1]");
        assert_eq!(fqn[1].index, BuilderFqnSegmentIndex::Index(1));
        assert_eq!(fqn[2].to_string(), "test");

        // test empty segments
        assert!(BuilderFqnSegment::try_from("").is_err());

        assert!(BuilderFqn::try_from("boomerang/fqn[1]/").is_err());

        assert_eq!(
            BuilderFqn::try_from("boomerang/fqn[1]/test").unwrap(),
            BuilderFqn::try_from("boomerang/fqn[1]/test").unwrap()
        );
    }

    /// Test the FqnSegment trait for ReactorBuilder
    #[test]
    fn test_fqn_segment_reactor_builder() {
        let reactor = ReactorBuilder::new("TestReactor", "", (), None, None, false);
        let segment = reactor.fqn_segment(false);
        assert_eq!(segment.to_string(), "TestReactor");

        let banked_reactor = ReactorBuilder::new(
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

    /// Test the FqnSegment trait for the ReactionBuilder
    #[test]
    fn test_fqn_segment_reaction_builder() {
        let reaction = ReactionBuilder::new(
            "TestReaction",
            0,
            BuilderReactorKey::default(),
            Box::new(|_| runtime::reaction_closure!().into()),
        );
        let segment = reaction.fqn_segment(false);
        assert_eq!(segment.to_string(), "TestReaction");

        // Test that the index is None for reactions
        assert_eq!(segment.index, BuilderFqnSegmentIndex::None);
    }

    /// Test the FqnSegment trait for ActionBuilder
    #[test]
    fn test_fqn_segment_action_builder() {
        let action = ActionBuilder::new(
            "TestAction",
            BuilderReactorKey::default(),
            crate::ActionType::Shutdown,
        );
        let segment = action.fqn_segment(false);
        assert_eq!(segment.to_string(), "TestAction");

        // Test that the index is None for actions
        assert_eq!(segment.index, BuilderFqnSegmentIndex::None);
    }

    /// Test the FqnSegment trait for PortBuilder
    #[test]
    fn test_fqn_segment_port_builder() {
        let port = PortBuilder::<(), Input>::new(
            "TestPort",
            BuilderReactorKey::default(),
            None, // No bank info
        );
        let segment = (&port as &dyn BasePortBuilder).fqn_segment(false);
        assert_eq!(segment.to_string(), "TestPort");

        // Test that the index is None for ports without bank info
        assert_eq!(segment.index, BuilderFqnSegmentIndex::None);

        // Test with bank info
        let port_banked = PortBuilder::<(), Input>::new(
            "BankedPort",
            BuilderReactorKey::default(),
            Some(runtime::BankInfo { idx: 0, total: 10 }),
        );
        let segment_banked = (&port_banked as &dyn BasePortBuilder).fqn_segment(false);
        assert_eq!(segment_banked.to_string(), "BankedPort[0]");

        let segment_banked_grouped = (&port_banked as &dyn BasePortBuilder).fqn_segment(true);
        assert_eq!(segment_banked_grouped.to_string(), "BankedPort[0..10]");
    }
}
