use std::{
    borrow::BorrowMut,
    ops::IndexMut,
    sync::{Arc, RwLock},
};

use super::{BuilderError, EnvBuilder, PortBuilder, PortType, ReactionBuilder};
use crate::runtime::{self};
use slotmap::{Key, SecondaryMap};

pub trait Reactor: Send + Sync + 'static {
    type Inputs;
    type Outputs;
    /// Build a new Reactor with the given instance name
    fn build(
        self,
        name: &str,
        env: &mut EnvBuilder,
        parent: Option<runtime::ReactorKey>,
    ) -> Result<(runtime::ReactorKey, Self::Inputs, Self::Outputs), BuilderError>;
}

// ------------------------- //

/// Reactor Prototype
#[derive(Debug)]
pub(super) struct ReactorBuilder {
    /// The instantiated/child name of the Reactor
    pub name: String,
    /// The top-level/class name of the Reactor
    pub type_name: String,
    /// Optional parent reactor key
    pub parent_reactor_key: Option<runtime::ReactorKey>,
    /// Reactions in this ReactorType
    pub reactions: SecondaryMap<runtime::ReactionKey, ()>,
    pub ports: SecondaryMap<runtime::BasePortKey, ()>,
    /* Child reactor instances declared on this ReactorBuilder
     * children: Vec<ReactorTypeChildRef>,
     * Port connections declared on this ReactorBuilder
     * connections: Vec<ReactorTypeConnection>, */
}

impl ReactorBuilder {
    fn new(name: &str, type_name: &str, parent_reactor_key: Option<runtime::ReactorKey>) -> Self {
        Self {
            name: name.into(),
            type_name: type_name.into(),
            parent_reactor_key,
            reactions: SecondaryMap::new(),
            ports: SecondaryMap::new(),
        }
    }
}

/// Builder struct used to facilitate construction of a ReactorType
#[derive(Debug)]
pub struct ReactorBuilderState<'a, R: Reactor> {
    reactor_key: runtime::ReactorKey,
    reactor: Arc<RwLock<R>>,
    env_builder: &'a mut EnvBuilder,
}

impl<'a, R: Reactor> ReactorBuilderState<'a, R> {
    pub(super) fn new(
        name: &str,
        parent: Option<runtime::ReactorKey>,
        reactor: R,
        env_builder: &'a mut EnvBuilder,
    ) -> Self {
        let reactor_key = env_builder.reactors.insert(ReactorBuilder::new(
            name,
            std::any::type_name::<R>(),
            parent,
        ));

        Self {
            reactor_key,
            reactor: Arc::new(RwLock::new(reactor)),
            env_builder,
        }
    }

    /// Add a new Port to this Reactor
    fn add_port<T: runtime::PortData>(
        &mut self,
        name: &str,
        port_type: PortType,
    ) -> Result<runtime::PortKey<T>, BuilderError> {
        // Ensure no duplicates
        if self
            .env_builder
            .port_builders
            .iter()
            .find(|(_, port)| {
                port.get_name() == name && port.get_reactor_key() == self.reactor_key
            })
            .is_some()
        {
            return Err(BuilderError::DuplicatedPortDefinition {
                reactor_name: self.env_builder.reactors[self.reactor_key].name.clone(),
                port_name: name.into(),
            });
        }

        let base_port_key = self.env_builder.ports.insert(Arc::new(runtime::Port::new(
            name.into(),
            runtime::PortValue::new(Option::<T>::None),
        )));
        let port_key = base_port_key.data().into();

        self.env_builder.port_builders.insert(
            base_port_key,
            Box::new(PortBuilder::<T>::new(
                name,
                port_key,
                self.reactor_key,
                port_type,
            )),
        );

        Ok(port_key)
    }

    pub fn add_input<T: runtime::PortData>(
        &mut self,
        name: &str,
    ) -> Result<runtime::PortKey<T>, BuilderError> {
        self.add_port(name, PortType::Input)
    }

    pub fn add_output<T: runtime::PortData>(
        &mut self,
        name: &str,
    ) -> Result<runtime::PortKey<T>, BuilderError> {
        self.add_port(name, PortType::Output)
    }

    pub fn add_startup_timer(&mut self, name: &str) -> runtime::BaseActionKey {
        self.add_timer(
            name,
            runtime::Duration::from_micros(0),
            runtime::Duration::from_micros(0),
        )
    }

    pub fn add_timer(
        &mut self,
        name: &str,
        period: runtime::Duration,
        offset: runtime::Duration,
    ) -> runtime::BaseActionKey {
        // let action_idx = ActionIndex(self.env.actions.len());
        // self.env.actions.push(ActionBuilder::new_timer_action(
        // name,
        // action_idx,
        // self.reactor_type_idx,
        // offset,
        // period,
        // ));
        // action_idx
        todo!()
    }

    pub fn add_reaction<F>(&mut self, reaction_fn: F) -> ReactionBuilder
    where
        F: Fn(&mut R, &runtime::SchedulerPoint) + Send + Sync + 'static,
    {
        // Priority = number of reactions declared thus far + 1
        let priority = self.env_builder.reactors[self.reactor_key].reactions.len();
        let reactor = self.reactor.clone();
        ReactionBuilder::new(
            std::any::type_name::<F>(),
            priority,
            self.reactor_key,
            runtime::ReactionFn::new(move |sched| {
                let mut reactor = reactor.write().unwrap();
                reaction_fn(&mut *reactor, sched);
            }),
        )
    }

    pub fn finish(self) -> Result<runtime::ReactorKey, BuilderError> {
        Ok(self.reactor_key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_input() {
        let mut env_builder = EnvBuilder::new();
        let mut builder_state = ReactorBuilder::new(
            "test_reactor",
            "test_ref",
            ReactorTypeBuilderIndex(0),
            None,
            None,
            ReactorTypeIndex(0),
            &mut env_builder,
        );

        let type_builder = ReactorTypeBuilder::new(
            "type_builder",
            ReactorTypeBuilderIndex(0),
            Box::new(|mut builder_state: ReactorBuilder| -> ReactorBuilder {
                builder_state.add_input::<u32>("p0").unwrap();
                assert_eq!(
                    builder_state
                        .add_input::<u32>("p0")
                        .expect_err("Expected duplicate"),
                    BuilderError::DuplicatedPortDefinition {
                        reactor_name: "test_reactor".into(),
                        port_name: "p0".into(),
                    }
                );
                builder_state.finish()
            }),
        );
    }
}