//! The `EnvBuilder` is the main entry point for building a Boomerang environment.
//!
//! The `EnvBuilder` is used to build a Boomerang environment, which is then used to create a
//! [`runtime::Env`]. The `EnvBuilder` is a builder pattern, where each step adds a new part to the
//! environment. The `EnvBuilder` is then converted into a [`runtime::Env`] using the
//! [`TryInto`] trait.

use super::{
    action::ActionBuilder, port::BasePortBuilder, reaction::ReactionBuilder, ActionBuilderFn,
    ActionType, BuilderActionKey, BuilderError, BuilderPortKey, BuilderReactionKey,
    BuilderReactorKey, Logical, Physical, PortBuilder, PortType, ReactionBuilderState,
    ReactorBuilder, ReactorBuilderState, TypedActionKey, TypedPortKey,
};
use crate::runtime;
use itertools::Itertools;
use slotmap::{SecondaryMap, SlotMap};
use std::{rc::Rc, sync::Arc, time::Duration};

mod debug;
#[cfg(feature = "federated")]
mod federated;
mod output;
#[cfg(test)]
mod tests;

pub trait FindElements {
    fn get_port_by_name(&self, port_name: &str) -> Result<BuilderPortKey, BuilderError>;

    fn get_action_by_name(&self, action_name: &str) -> Result<BuilderActionKey, BuilderError>;
}

#[derive(Default)]
pub struct EnvBuilder {
    /// Builders for Ports
    pub(super) port_builders: SlotMap<BuilderPortKey, Box<dyn BasePortBuilder>>,
    /// Builders for Reactions
    pub(super) reaction_builders: SlotMap<BuilderReactionKey, ReactionBuilder>,
    /// Builders for Reactors
    pub(super) reactor_builders: SlotMap<BuilderReactorKey, ReactorBuilder>,
}

/// Methods for populating the `EnvBuilder` with new parts.
impl EnvBuilder {
    pub fn new() -> Self {
        Default::default()
    }

    /// Add a new Reactor
    pub fn add_reactor<S: runtime::ReactorState>(
        &mut self,
        name: &str,
        parent: Option<BuilderReactorKey>,
        reactor: S,
    ) -> ReactorBuilderState {
        ReactorBuilderState::new(name, parent, reactor, self)
    }

    /// Add a new Reactor by cloning an existing one
    pub fn add_reactor_cloned(
        &mut self,
        name: &str,
        parent: Option<BuilderReactorKey>,
        reactor_key: BuilderReactorKey,
    ) -> ReactorBuilderState {
        let existing = &self.reactor_builders[reactor_key];

        let builder = ReactorBuilder {
            name: name.into(),
            state: existing.state.clone(),
            type_name: existing.type_name.clone(),
            parent_reactor_key: parent,
            reactions: SecondaryMap::new(),
            ports: SecondaryMap::new(),
            actions: SlotMap::with_key(),
        };

        ReactorBuilderState::from_reactor(builder, self)
    }

    /// Add a new Port
    pub fn add_port<T: runtime::PortData>(
        &mut self,
        name: &str,
        port_type: PortType,
        reactor_key: BuilderReactorKey,
    ) -> Result<TypedPortKey<T>, BuilderError> {
        // Ensure no duplicates
        if self
            .port_builders
            .values()
            .any(|port| port.get_name() == name && port.get_reactor_key() == reactor_key)
        {
            return Err(BuilderError::DuplicatePortDefinition {
                reactor_name: self.reactor_builders[reactor_key].get_name().to_owned(),
                port_name: name.into(),
            });
        }

        let key = self.port_builders.insert_with_key(|port_key| {
            self.reactor_builders[reactor_key]
                .ports
                .insert(port_key, ());
            Box::new(PortBuilder::<T>::new(name, reactor_key, port_type))
        });

        Ok(TypedPortKey::new(key))
    }

    /// Add a new Startup Action
    pub fn add_startup_action(
        &mut self,
        name: &str,
        reactor_key: BuilderReactorKey,
    ) -> Result<TypedActionKey, BuilderError> {
        self.add_action(
            name,
            ActionType::Startup,
            reactor_key,
            Rc::new(|_: &'_ str, _: runtime::keys::ActionKey| runtime::Action::Startup),
        )
        .map(|key| key.into())
    }

    /// Add a new Shutdown Action
    pub fn add_shutdown_action(
        &mut self,
        name: &str,
        reactor_key: BuilderReactorKey,
    ) -> Result<TypedActionKey, BuilderError> {
        self.add_action(
            name,
            ActionType::Shutdown,
            reactor_key,
            Rc::new(|_: &'_ str, _: runtime::keys::ActionKey| runtime::Action::Shutdown),
        )
        .map(|key| key.into())
    }

    /// Add a new Logical Action
    pub fn add_logical_action<T: runtime::ActionData>(
        &mut self,
        name: &str,
        min_delay: Option<Duration>,
        reactor_key: BuilderReactorKey,
    ) -> Result<TypedActionKey<T, Logical>, BuilderError> {
        self.add_action(
            name,
            ActionType::Logical { min_delay },
            reactor_key,
            Rc::new(move |name: &'_ str, key: runtime::keys::ActionKey| {
                runtime::Action::Logical(runtime::LogicalAction::new::<T>(
                    name,
                    key,
                    min_delay.unwrap_or_default(),
                ))
            }),
        )
        .map(|key| key.into())
    }

    /// Add a new Physical Action
    pub fn add_physical_action<T: runtime::ActionData>(
        &mut self,
        name: &str,
        min_delay: Option<Duration>,
        reactor_key: BuilderReactorKey,
    ) -> Result<TypedActionKey<T, Physical>, BuilderError> {
        self.add_action(
            name,
            ActionType::Physical { min_delay },
            reactor_key,
            Rc::new(move |name: &'_ str, action_key| {
                runtime::Action::Physical(runtime::PhysicalAction::new::<T>(
                    name,
                    action_key,
                    min_delay.unwrap_or_default(),
                ))
            }),
        )
        .map(|key| key.into())
    }

    /// Adds a new logical action with the same underlying type as the port.
    pub fn add_logical_action_from_port(
        &mut self,
        name: &str,
        port_key: BuilderPortKey,
        reactor_key: BuilderReactorKey,
    ) -> Result<BuilderActionKey, BuilderError> {
        self.add_action(
            name,
            ActionType::Logical { min_delay: None },
            reactor_key,
            self.port_builders[port_key].create_same_typed_action_builder(None),
        )
    }

    /// Add a Reaction to a given Reactor
    pub fn add_reaction(
        &mut self,
        name: &str,
        reactor_key: BuilderReactorKey,
        reaction_fn: Arc<dyn runtime::ReactionFn>,
    ) -> ReactionBuilderState {
        let priority = self.reactor_builders[reactor_key].reactions.len();
        ReactionBuilderState::new(name, priority, reactor_key, reaction_fn, self)
    }

    /// Add an Action generically to a given Reactor using the given `ActionBuilderFn`
    pub fn add_action(
        &mut self,
        name: &str,
        ty: ActionType,
        reactor_key: BuilderReactorKey,
        action_fn: Rc<dyn ActionBuilderFn>,
    ) -> Result<BuilderActionKey, BuilderError> {
        let reactor_builder = &mut self.reactor_builders[reactor_key];

        // Ensure no duplicates
        if reactor_builder
            .actions
            .values()
            .any(|action_builder| action_builder.get_name() == name)
        {
            return Err(BuilderError::DuplicateActionDefinition {
                reactor_name: self.reactor_builders[reactor_key].get_name().to_owned(),
                action_name: name.into(),
            });
        }

        let key = reactor_builder
            .actions
            .insert(ActionBuilder::new(name, ty, action_fn));

        Ok(key)
    }

    /// Bind Port A to Port B
    /// The nominal case is to bind Input A to Output B
    pub fn bind_port<T: runtime::PortData>(
        &mut self,
        port_a_key: TypedPortKey<T>,
        port_b_key: TypedPortKey<T>,
    ) -> Result<(), BuilderError> {
        let port_a_key = port_a_key.into();
        let port_b_key = port_b_key.into();

        let port_a = &self.port_builders[port_a_key];
        let port_b = &self.port_builders[port_b_key];

        if port_b.get_inward_binding().is_some() {
            return Err(BuilderError::PortBindError {
                port_a_key,
                port_b_key,
                what: format!(
                    "Ports may only be connected once, but B is already connected to {:?}",
                    port_b.get_inward_binding()
                ),
            });
        }

        if !port_a.get_deps().is_empty() {
            return Err(BuilderError::PortBindError {
                port_a_key,
                port_b_key,
                what: "Ports with dependencies may not be connected to other ports".to_owned(),
            });
        }

        if port_b.get_antideps().len() > 0 {
            return Err(BuilderError::PortBindError {
                port_a_key,
                port_b_key,
                what: "Ports with antidependencies may not be connected to other ports".to_owned(),
            });
        }

        match (port_a.get_port_type(), port_b.get_port_type()) {
            (PortType::Input, PortType::Input) => {
                self.reactor_builders[port_b.get_reactor_key()]
                    .parent_reactor_key
                    .and_then(|parent_key| {
                        port_a.get_reactor_key().eq(&parent_key).then_some(())
                     }).ok_or(
                        BuilderError::PortBindError {
                            port_a_key,
                            port_b_key,
                            what: "An input port A may only be bound to another input port B if B is contained by a reactor that in turn is contained by the reactor of A.".into()
                        })
            }
            (PortType::Output, PortType::Input) => {
                let port_a_grandparent =
                    self.reactor_builders[port_a.get_reactor_key()].parent_reactor_key;
                let port_b_grandparent =
                    self.reactor_builders[port_b.get_reactor_key()].parent_reactor_key;
                // VALIDATE(this->container()->container() == port->container()->container(),
                if !matches!((port_a_grandparent, port_b_grandparent), (Some(key_a), Some(key_b)) if key_a == key_b)
                {
                    let port_a_fqn = self.port_fqn(port_a_key)?;
                    let port_b_fqn = self.port_fqn(port_b_key)?;

                    Err(BuilderError::PortBindError {
                        port_a_key,
                        port_b_key,
                        what: format!("An output port ({port_a_fqn}) can only be bound to an input port ({port_b_fqn}) if both ports belong to reactors in the same hierarichal level"),
                    })
                }
                // VALIDATE(this->container() != port->container(), );
                else if port_a.get_reactor_key() == port_b.get_reactor_key() {
                    let port_a_fqn = self.port_fqn(port_a_key)?;
                    let port_b_fqn = self.port_fqn(port_b_key)?;

                    Err(BuilderError::PortBindError {
                        port_a_key,
                        port_b_key,
                        what: format!("An output port ({port_a_fqn}) can only be bound to an input port ({port_b_fqn}) if both ports belong to different reactors!"),
                    })
                } else {
                    Ok(())
                }
            }
            (PortType::Output, PortType::Output) => {
                // VALIDATE( this->container()->container() == port->container(),
                self.reactor_builders[port_a.get_reactor_key()]
                    .parent_reactor_key
                    .and_then(|parent_key| {
                        if parent_key == port_b.get_reactor_key() {
                            Some(())
                        } else {
                            None
                        }
                    }).ok_or(
                        BuilderError::PortBindError {
                                port_a_key,
                                port_b_key,
                                what: "An output port A may only be bound to another output port B if A is contained by a reactor that in turn is contained by the reactor of B".to_owned()
                            })
            }
            (PortType::Input, PortType::Output) => Err(BuilderError::PortBindError {
                port_a_key,
                port_b_key,
                what: "Unexpected case: can't bind an input Port to an output Port.".to_owned(),
            }),
        }?;

        // All validity checks passed, so we can now bind the ports
        self.port_builders[port_b_key].set_inward_binding(Some(port_a_key));
        self.port_builders[port_a_key].add_outward_binding(port_b_key);

        Ok(())
    }

    /// Recursively remove a reactor from the environment
    pub fn remove_reactor(&mut self, reactor_key: BuilderReactorKey) -> Result<(), BuilderError> {
        tracing::debug!(
            ?reactor_key,
            "Removing reactor {}",
            self.reactor_fqn(reactor_key).unwrap()
        );

        // Remove any children
        let children_keys = self
            .reactor_builders
            .iter()
            .filter_map(|(key, builder)| {
                builder
                    .parent_reactor_key
                    .and_then(|parent_key| (parent_key == reactor_key).then_some(key))
            })
            .collect_vec();

        for key in children_keys {
            self.remove_reactor(key)?;
        }

        // Remove any ports in the reactor
        let removed_ports = self
            .port_builders
            .iter()
            .filter_map(|(port_key, port)| {
                (port.get_reactor_key() == reactor_key).then(|| port_key)
            })
            .collect_vec();

        for port_key in removed_ports.iter() {
            tracing::debug!(
                ?port_key,
                "Removing port {}",
                self.port_fqn(*port_key).unwrap()
            );
            self.port_builders.remove(*port_key);
        }

        // Remove any dangling port bindings
        for (_, port) in self.port_builders.iter_mut() {
            for removed in removed_ports.iter() {
                port.clear_bindings_to(*removed);
            }
        }

        for (_, reaction) in self.reaction_builders.iter_mut() {
            for removed in removed_ports.iter() {
                reaction.input_ports.remove(*removed);
                reaction.output_ports.remove(*removed);
            }
        }

        // Remove any reactions in the reactor
        let removed_reactions = self
            .reaction_builders
            .iter()
            .filter_map(|(reaction_key, reaction)| {
                (reaction.reactor_key == reactor_key).then(|| reaction_key)
            })
            .collect_vec();

        for reaction_key in removed_reactions.iter() {
            tracing::debug!(
                ?reaction_key,
                "Removing reaction {}",
                self.reaction_fqn(*reaction_key).unwrap()
            );
            self.reaction_builders.remove(*reaction_key);
        }

        self.reactor_builders.remove(reactor_key);

        Ok(())
    }
}

/// Introspection methods for the environment
impl EnvBuilder {
    /// Find Reactors matching a given name
    pub fn find_reactors_by_name<'a>(
        &'a self,
        reactor_name: &'a str,
    ) -> impl Iterator<Item = BuilderReactorKey> + 'a {
        self.reactor_builders
            .iter()
            .filter_map(move |(reactor_key, reactor)| {
                (reactor.get_name() == reactor_name).then(|| reactor_key)
            })
    }

    /// Find a Port matching a given name and ReactorKey
    pub fn find_port_by_name(
        &self,
        port_name: &str,
        reactor_key: BuilderReactorKey,
    ) -> Result<BuilderPortKey, BuilderError> {
        self.port_builders
            .iter()
            .find(|(_, port_builder)| {
                port_builder.get_name() == port_name
                    && port_builder.get_reactor_key() == reactor_key
            })
            .map(|(port_key, _)| port_key)
            .ok_or_else(|| BuilderError::NamedPortNotFound(port_name.to_string()))
    }

    /// Find an Action matching a given name and ReactorKey
    pub fn find_action_by_name(
        &self,
        action_name: &str,
        reactor_key: BuilderReactorKey,
    ) -> Result<BuilderActionKey, BuilderError> {
        self.reactor_builders[reactor_key]
            .actions
            .iter()
            .find(|(_, action_builder)| action_builder.get_name() == action_name)
            .map(|(action_key, _)| action_key)
            .ok_or_else(|| BuilderError::NamedActionNotFound(action_name.to_string()))
    }
}
