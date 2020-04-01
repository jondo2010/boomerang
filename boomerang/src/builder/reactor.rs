use super::{
    ActionBuilder, BuilderError, EnvBuilderState, PortBuilder,
    PortType, ReactionBuilder, ReactorTypeBuilderChildRefIndex, ReactorTypeBuilderIndex,
    ReactorTypeIndex,
};
use crate::runtime::{self, ActionIndex, PortIndex, ReactionIndex};

#[derive(Debug, Clone)]
pub struct ReactorTypeChildRef {
    pub name: String,
    pub reactor_type_builder_idx: ReactorTypeBuilderIndex,
    /// Possible index of the parent ReactorType
    pub parent_type_idx: Option<ReactorTypeIndex>,
    /// Possible index of this child in the parent
    pub parent_builder_child_ref_idx: Option<ReactorTypeBuilderChildRefIndex>,
}

/// Reactor Prototype
#[derive(Debug)]
pub struct ReactorType {
    pub name: String,
    /// The parent/owning ReactorType
    pub parent_reactor_type_idx: Option<ReactorTypeIndex>,
    /// What ref_idx resulted in this ReactorType
    pub builder_ref_idx: ReactorTypeBuilderIndex,
    /// Index into the parents' vector of ReactorTypeChildRef
    pub parent_builder_child_ref_idx: Option<ReactorTypeBuilderChildRefIndex>,
    /// Reactions in this ReactorType
    pub reactions: Vec<ReactionIndex>,
    pub ports: Vec<PortIndex>,
    pub children: Vec<ReactorTypeChildRef>,
    pub connections: Vec<ReactorTypeConnection>,
}

/// Callback function used to build a ReactorType
type ReactorTypeBuilderFn = dyn Fn(ReactorTypeBuilderState) -> ReactorType;

/// A ReactorTypeBuilder is a declaration of a named builder function that produces a ReactorType
pub(crate) struct ReactorTypeBuilder {
    name: String,
    reactor_type_builder_idx: ReactorTypeBuilderIndex,
    builder_fn: Box<ReactorTypeBuilderFn>,
}

impl std::fmt::Debug for ReactorTypeBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReactorProtoBuilderRef")
            .field("name", &self.name)
            .field("reactor_type_builder_idx", &self.reactor_type_builder_idx)
            .field("builder_fn", &"Box<ReactorTypeBuilderFn>")
            .finish()
    }
}

impl ReactorTypeBuilder {
    pub fn new(
        name: &str,
        reactor_type_builder_idx: ReactorTypeBuilderIndex,
        builder_fn: Box<ReactorTypeBuilderFn>,
    ) -> Self {
        Self {
            name: name.into(),
            reactor_type_builder_idx,
            builder_fn,
        }
    }
    pub fn build(
        &mut self,
        env_builder_state: &mut EnvBuilderState,
        reactor_type_child_ref: &ReactorTypeChildRef,
    ) -> ReactorType {
        let reactor_type_idx = ReactorTypeIndex(env_builder_state.expanded_reactor_types.len());
        let proto = (self.builder_fn)(ReactorTypeBuilderState::new(
            &reactor_type_child_ref.name,
            &self.name,
            reactor_type_child_ref.reactor_type_builder_idx,
            reactor_type_child_ref.parent_type_idx,
            reactor_type_child_ref.parent_builder_child_ref_idx,
            reactor_type_idx,
            env_builder_state,
        ));
        proto
    }
}

#[derive(Debug)]
pub struct ReactorTypeConnection {
    pub from_reactor_idx: ReactorTypeBuilderChildRefIndex,
    pub from_port: String,
    pub to_reactor_idx: ReactorTypeBuilderChildRefIndex,
    pub to_port: String,
}

/// Builder struct used to facilitate construction of a ReactorType
/// This gets passed into the builder callback.
#[derive(Debug)]
pub struct ReactorTypeBuilderState<'a> {
    /// The instantiated/child name of the ReactorProto
    pub name: String,
    /// The top-level/class name of the ReactorBuilderRef
    ref_name: String,
    /// The index of the BuilderRef that is referencing this ReactorTypeBuilder
    reactor_type_builder_idx: ReactorTypeBuilderIndex,
    /// The parent ReactorProto
    parent_reactor_type_idx: Option<ReactorTypeIndex>,
    /// Index into the parents' vector of ReactorTypeChildRef
    parent_builder_child_ref_idx: Option<ReactorTypeBuilderChildRefIndex>,
    /// Carry the index of the ReactorProto we'll be inserted into
    reactor_type_idx: ReactorTypeIndex,
    /// Reactions declared on this ReactorBuilder
    reactions: Vec<ReactionIndex>,
    /// Ports declared on this ReactorBuilder
    ports: Vec<PortIndex>,
    /// Child reactor instances declared on this ReactorBuilder
    children: Vec<ReactorTypeChildRef>,
    /// Port connections declared on this ReactorBuilder
    connections: Vec<ReactorTypeConnection>,
    /// A mutable reference to the top-level state.
    env: &'a mut EnvBuilderState,
}

impl<'a> ReactorTypeBuilderState<'a> {
    pub fn new(
        name: &str,
        ref_name: &str,
        reactor_type_builder_idx: ReactorTypeBuilderIndex,
        parent_reactor_type_idx: Option<ReactorTypeIndex>,
        parent_builder_child_ref_idx: Option<ReactorTypeBuilderChildRefIndex>,
        reactor_type_idx: ReactorTypeIndex,
        env: &'a mut EnvBuilderState,
    ) -> Self {
        ReactorTypeBuilderState {
            name: name.into(),
            ref_name: ref_name.into(),
            reactor_type_builder_idx,
            parent_reactor_type_idx,
            parent_builder_child_ref_idx,
            reactor_type_idx,
            reactions: Vec::new(),
            ports: Vec::new(),
            children: Vec::new(),
            connections: Vec::new(),
            env,
        }
    }

    pub fn add_input<T: runtime::PortData>(
        &mut self,
        name: &str,
    ) -> Result<PortIndex, BuilderError> {
        // Ensure no duplicates
        if self
            .env
            .ports
            .iter()
            .find(|&port| {
                port.get_name() == name && port.get_reactor_type_idx() == self.reactor_type_idx
            })
            .is_some()
        {
            return Err(BuilderError::DuplicatedPortDefinition {
                reactor_name: self.name.clone(),
                port_name: name.into(),
            });
        }

        let idx = PortIndex(self.env.ports.len());
        self.ports.push(idx);
        self.env.ports.push(Box::new(PortBuilder::<T>::new(
            //&format!("{}.{}", self.name, name),
            name,
            self.reactor_type_idx,
            PortType::Input,
        )));
        Ok(idx)
    }

    pub fn add_output<T: runtime::PortData>(
        &mut self,
        name: &str,
    ) -> Result<PortIndex, BuilderError> {
        // Ensure no duplicates
        if self
            .env
            .ports
            .iter()
            .find(|&port| {
                port.get_name() == name && port.get_reactor_type_idx() == self.reactor_type_idx
            })
            .is_some()
        {
            return Err(BuilderError::DuplicatedPortDefinition {
                reactor_name: self.name.clone(),
                port_name: name.into(),
            });
        }
        let idx = PortIndex(self.env.ports.len());
        self.ports.push(idx);
        self.env.ports.push(Box::new(PortBuilder::<T>::new(
            //&format!("{}.{}", self.name, name),
            name,
            self.reactor_type_idx,
            PortType::Output,
        )));
        Ok(idx)
    }

    pub fn add_startup_timer(&mut self, name: &str) -> ActionIndex {
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
    ) -> ActionIndex {
        let action_idx = ActionIndex(self.env.actions.len());
        self.env.actions.push(ActionBuilder::new_timer_action(
            name,
            action_idx,
            self.reactor_type_idx,
            offset,
            period,
        ));
        action_idx
    }

    pub fn add_reaction<F>(&mut self, name: &str, reaction_fn: F) -> ReactionBuilder
    where
        F: FnMut(&runtime::SchedulerPoint) + Send + Sync + 'static,
    {
        let reaction_idx = ReactionIndex(self.env.reactions.len());
        self.reactions.push(reaction_idx);
        // Priority = number of reactions declared thus far + 1
        ReactionBuilder::new(
            name,
            self.reactions.len(),
            reaction_idx,
            self.reactor_type_idx,
            runtime::ReactionFn::new(reaction_fn),
            self.env,
        )
    }

    pub fn add_child_instance(
        &mut self,
        name: &str,
        reactor_type_builder_idx: ReactorTypeBuilderIndex,
    ) -> ReactorTypeBuilderChildRefIndex {
        let idx = ReactorTypeBuilderChildRefIndex(self.children.len());
        self.children.push(ReactorTypeChildRef {
            // name: format!("{}.{}", &self.name, name),
            name: name.into(),
            reactor_type_builder_idx: reactor_type_builder_idx,
            parent_type_idx: Some(self.reactor_type_idx),
            parent_builder_child_ref_idx: Some(idx),
        });
        idx
    }

    pub fn add_connection(
        &mut self,
        from_reactor_idx: ReactorTypeBuilderChildRefIndex,
        from_port: &str,
        to_reactor_idx: ReactorTypeBuilderChildRefIndex,
        to_port: &str,
    ) {
        self.connections.push(ReactorTypeConnection {
            from_reactor_idx: from_reactor_idx,
            from_port: from_port.into(),
            to_reactor_idx: to_reactor_idx,
            to_port: to_port.into(),
        })
    }

    pub fn finish(self) -> ReactorType {
        ReactorType {
            name: self.name,
            parent_reactor_type_idx: self.parent_reactor_type_idx,
            builder_ref_idx: self.reactor_type_builder_idx,
            parent_builder_child_ref_idx: self.parent_builder_child_ref_idx,
            reactions: self.reactions,
            ports: self.ports,
            children: self.children,
            connections: self.connections,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_input() {
        let mut env_builder = EnvBuilderState::new();
        let mut builder_state = ReactorTypeBuilderState::new(
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
            Box::new(
                |mut builder_state: ReactorTypeBuilderState| -> ReactorType {
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
                },
            ),
        );
    }
}
