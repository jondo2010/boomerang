use super::{Env, ReactionGraph};
use crate::fmt_utils as fmt;

impl std::fmt::Debug for Env {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let reactors = fmt::from_fn(|f| {
            f.debug_map()
                .entries(
                    self.reactors
                        .iter()
                        .map(|(k, reactor)| (format!("{k:?}"), reactor.get_name())),
                )
                .finish()
        });

        let actions = fmt::from_fn(|f| {
            let e = self
                .actions
                .iter()
                .map(|(action_key, action)| (format!("{action_key:?}"), action.to_string()));
            f.debug_map().entries(e).finish()
        });

        let ports = fmt::from_fn(|f| {
            f.debug_map()
                .entries(
                    self.ports
                        .iter()
                        .map(|(k, v)| (format!("{k:?}"), v.to_string())),
                )
                .finish()
        });

        let reactions = fmt::from_fn(|f| {
            f.debug_map()
                .entries(
                    self.reactions
                        .iter()
                        .map(|(reaction_key, reaction)| (format!("{reaction_key:?}"), reaction)),
                )
                .finish()
        });

        f.debug_struct("Env")
            .field("reactors", &reactors)
            .field("actions", &actions)
            .field("ports", &ports)
            .field("reactions", &reactions)
            .finish()
    }
}

impl std::fmt::Debug for ReactionGraph {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let action_triggers = fmt::from_fn(|f| {
            let e = self.action_triggers.iter().map(|(action_key, v)| {
                let v = fmt::from_fn(|f| {
                    let e = v.iter().map(|(level, reaction_key)| {
                        (format!("{level:?}"), format!("{reaction_key:?}"))
                    });
                    f.debug_map().entries(e).finish()
                });

                (format!("{action_key:?}"), v)
            });
            f.debug_map().entries(e).finish()
        });

        let port_triggers = fmt::from_fn(|f| {
            let e = self.port_triggers.iter().map(|(port_key, v)| {
                let v = fmt::from_fn(|f| {
                    let e = v.iter().map(|(level, reaction_key)| {
                        (format!("{level:?}"), format!("{reaction_key:?}"))
                    });
                    f.debug_map().entries(e).finish()
                });

                (format!("{port_key:?}"), v)
            });
            f.debug_map().entries(e).finish()
        });

        f.debug_struct("TriggerMap")
            .field("action_triggers", &action_triggers)
            .field("port_triggers", &port_triggers)
            .field("startup_reactions", &self.startup_reactions)
            .field("shutdown_reactions", &self.shutdown_reactions)
            .field("reaction_set_limits", &self.reaction_set_limits)
            .field("reaction_use_ports", &self.reaction_use_ports)
            .field("reaction_effect_ports", &self.reaction_effect_ports)
            .field("reaction_actions", &self.reaction_actions)
            .field("reactor_bank_infos", &self.reactor_bank_infos)
            .finish()
    }
}
