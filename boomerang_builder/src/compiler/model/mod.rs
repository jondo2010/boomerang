//! Modular target-neutral structural application model.

mod action;
mod bank;
mod component;
mod connection;
mod enclave;
mod mode;
mod placement_group;
mod port;
mod reaction;
mod reactor;
mod topology;

pub use action::{Action, ActionKind};
pub use bank::{BankMember, InvalidBankMember};
pub use component::ComponentInstance;
pub use connection::{Connection, ConnectionSemantics};
pub use enclave::Enclave;
pub use mode::Mode;
pub use placement_group::PlacementGroup;
pub use port::{Port, PortDirection};
pub use reaction::{
    ModeTransition, ModeTransitionKind, Reaction, ReactionOptions, ReactionRelation,
    ReactionRelationFlags, ReactionRelationTarget,
};
pub use reactor::Reactor;
pub use topology::{ApplicationTopology, ApplicationTopologyBuilder, TopologyBuildError};
