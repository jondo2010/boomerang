use super::{
    ActionIndex, BaseAction, BasePort, Port, PortData, PortIndex, Reaction, ReactionIndex,
};
use downcast_rs::{Downcast, DowncastSync};
use std::{collections::BTreeMap, sync::Arc};

//#[derive(Debug)]
// pub struct Environment {
//    scheduler: Scheduler,
//    run_forever: bool,
//    fast_fwd_execution: bool,
//}

/// Builder struct used to facilitate construction of a Reaction
/// This gets passed into the builder callback.
#[derive(Debug)]
pub struct Environment {
    pub runtime_ports: BTreeMap<PortIndex, Arc<dyn BasePort>>,
    pub runtime_reactions: BTreeMap<ReactionIndex, Arc<Reaction>>,
    pub runtime_actions: BTreeMap<ActionIndex, Arc<dyn BaseAction>>,
}

impl Environment {
    pub fn new() -> Self {
        Self {
            runtime_ports: BTreeMap::new(),
            runtime_reactions: BTreeMap::new(),
            runtime_actions: BTreeMap::new(),
        }
    }

    pub fn get_port<T>(&self, port_idx: PortIndex) -> Option<Arc<Port<T>>>
    where
        T: PortData,
    {
        self.runtime_ports
            .get(&port_idx)
            .cloned()
            .and_then(|base_port| DowncastSync::into_any_arc(base_port).downcast().ok())
    }

    /// Return the maximum reaction level
    pub fn max_level(&self) -> usize {
        self.runtime_reactions
            .values()
            .map(|reaction| reaction.get_level())
            .max()
            .unwrap_or_default()
    }
}
