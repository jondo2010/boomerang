use super::{
    BaseAction, BaseActionKey, BasePort, BasePortKey, Port, PortData, PortKey, Reaction,
    ReactionKey,
};
use downcast_rs::DowncastSync;
use slotmap::{Key, SecondaryMap, SlotMap};
use std::{fmt::Display, sync::Arc};

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
    /// The runtime set of Ports
    pub ports: SlotMap<BasePortKey, Arc<dyn BasePort>>,
    /// For each Port a set of Reactions triggered by it
    pub port_triggers: SecondaryMap<BasePortKey, SecondaryMap<ReactionKey, ()>>,
    /// The runtime set of Actions
    pub actions: SlotMap<BaseActionKey, Arc<dyn BaseAction>>,
    /// For each Action, a set of Reactions triggered by it
    pub action_triggers: SecondaryMap<BaseActionKey, SecondaryMap<ReactionKey, ()>>,
    /// The runtime set of Reactions
    pub reactions: SlotMap<ReactionKey, Arc<Reaction>>,
}

impl Display for Environment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Environment {\n")?;
        f.write_str("}\n")?;
        Ok(())
    }
}

impl Environment {
    pub fn new() -> Self {
        Self {
            ports: SlotMap::with_key(),
            port_triggers: SecondaryMap::new(),
            actions: SlotMap::with_key(),
            action_triggers: SecondaryMap::new(),
            reactions: SlotMap::with_key(),
        }
    }

    pub fn get_port<T: PortData>(&self, port_key: PortKey<T>) -> Option<Arc<Port<T>>>
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
}
