//! Crate-private scheduler coordination boundaries for owned Federates.

mod dependencies;
mod quiescence;

pub(crate) use dependencies::EnclaveDependencies;
pub(crate) use quiescence::{
    FederateQuiescence, FederateQuiescenceCoordinator, FederateQuiescenceHandle, QuiescenceControl,
    QuiescenceParticipant,
};
