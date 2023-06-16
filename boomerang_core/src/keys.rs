//! Runtime keys for the various types of Reactor components.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

tinymap::key_type! {
    /// Runtime key for a Reactor
    #[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
    pub ReactorKey
}

tinymap::key_type!(
    /// Runtime key for a Reaction
    #[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
    pub ReactionKey
);

tinymap::key_type!(
    /// Runtime key for a Port, unique to a Reactor hierarchy.
    #[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
    pub PortKey
);

tinymap::key_type! {
    /// Runtime key for an Action, unique to a Reactor
    #[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
    pub ActionKey
}
