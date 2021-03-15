use crate::SchedulerPoint;

use super::{
    BaseAction, ActionKey, BasePort, PortKey, Port, PortData, Reaction,
    ReactionKey,
};
use downcast_rs::DowncastSync;
use slotmap::{Key, SecondaryMap, SlotMap};
use std::{fmt::Display, sync::Arc};
use tracing::event;

/// Builder struct used to facilitate construction of a Reaction
/// This gets passed into the builder callback.
#[derive(Debug)]
pub struct Env<S: SchedulerPoint> {
    /// The runtime set of Ports
    pub ports: SlotMap<PortKey, Arc<dyn BasePort>>,
    /// For each Port a set of Reactions triggered by it
    pub port_triggers: SecondaryMap<PortKey, SecondaryMap<ReactionKey, ()>>,
    /// The runtime set of Actions
    pub actions: SlotMap<ActionKey, Arc<dyn BaseAction<S>>>,
    /// For each Action, a set of Reactions triggered by it
    pub action_triggers: SecondaryMap<ActionKey, SecondaryMap<ReactionKey, ()>>,
    /// The runtime set of Reactions
    pub reactions: SlotMap<ReactionKey, Reaction<S>>,
}

impl<S: SchedulerPoint> Display for Env<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Environment {\n")?;
        f.write_str("}\n")?;
        Ok(())
    }
}

impl<S: SchedulerPoint> Env<S> {
    pub fn new() -> Self {
        Self {
            ports: SlotMap::with_key(),
            port_triggers: SecondaryMap::new(),
            actions: SlotMap::with_key(),
            action_triggers: SecondaryMap::new(),
            reactions: SlotMap::with_key(),
        }
    }

    pub fn get_port<T: PortData>(&self, port_key: PortKey) -> Option<Arc<Port<T>>>
    where
        T: PortData,
    {
        self.ports
            .get(port_key.data().into())
            .cloned()
            .and_then(|base_port| DowncastSync::into_any_arc(base_port).downcast().ok())
    }

    /// Return the maximum reaction level
    pub fn max_level(&self) -> usize {
        self.reactions
            .values()
            .map(|reaction| reaction.get_level())
            .max()
            .unwrap_or_default()
    }

    pub fn startup(&self, sched: &S) {
        event!(tracing::Level::INFO, "Starting the execution");
        for action in self.actions.values() {
            action.startup(&sched);
        }
    }

    pub fn shutdown(&self, sched: &S) {
        event!(tracing::Level::INFO, "Terminating the execution");
        for action in self.actions.values() {
            action.shutdown(&sched);
        }
    }
}
