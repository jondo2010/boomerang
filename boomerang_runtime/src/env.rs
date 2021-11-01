use crate::{RuntimeError, SchedulerPoint};

use super::{ActionKey, BaseAction, BasePort, Port, PortData, PortKey, Reaction, ReactionKey};
use downcast_rs::DowncastSync;
use slotmap::{SecondaryMap, SlotMap};
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

    pub fn get_port<T: PortData>(&self, port_key: PortKey) -> Result<Arc<Port<T>>, RuntimeError>
    where
        T: PortData,
    {
        self.ports
            .get(port_key)
            .cloned()
            .ok_or(RuntimeError::PortKeyNotFound(port_key))
            .and_then(|base_port| {
                let base_type = base_port.type_name();
                DowncastSync::into_any_arc(base_port)
                    .downcast()
                    .map_err(|_err| RuntimeError::TypeMismatch {
                        found: base_type,
                        wanted: std::any::type_name::<T>(),
                    })
            })
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
        for (key, action) in self.actions.iter() {
            action.startup(&sched, key);
        }
    }

    pub fn shutdown(&self, sched: &S) {
        event!(tracing::Level::INFO, "Terminating the execution");
        for (key, action) in self.actions.iter() {
            action.shutdown(&sched, key);
        }
    }
}
