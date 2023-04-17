use std::sync::Arc;

use itertools::Itertools;

use crate::{
    builder::{BuilderError, BuilderPortKey, BuilderReactorKey, PortType, ReactorBuilderState},
    runtime,
};

use super::EnvBuilder;

impl<'a> ReactorBuilderState<'a> {
    /// For each outward binding on a port in the source `reactor`, create:
    /// 1. a reaction to forward it with `send_timed_message`, and a
    /// 2. a reaction sensitive to the outputControlReactonTrigger that will call `send_port_absent` if the port isn't set.
    fn transform_outward_bindings(
        &mut self,
        outward_bindings: &[(BuilderPortKey, String, BuilderPortKey, String)],
    ) -> Result<(), BuilderError> {
        let output_control_trigger =
            self.add_logical_action::<()>("outputControlReactionTrigger", None)?;

        for (from_port, from_port_name, to_port, to_port_name) in outward_bindings.iter() {
            let output_binding_reaction = {
                let to_port = *to_port;
                Arc::new(
                    move |_ctx: &mut runtime::Context,
                          _state: &mut dyn runtime::ReactorState,
                          inputs: &[runtime::IPort],
                          _outputs: &mut [runtime::OPort],
                          _actions: &mut [&mut runtime::Action]| {
                        let [port]: [runtime::IPort; 1] = inputs.try_into().unwrap();
                        if port.is_set() {
                            println!("send_timed_message({:?})", to_port)
                        }
                    },
                )
            };

            let _ = self
                .add_reaction(
                    &(format!("{from_port_name}_{to_port_name}")),
                    output_binding_reaction,
                )
                .with_trigger_port(*from_port, 0)
                .finish()?;

            let output_control_reaction = {
                let to_port = *to_port;
                Arc::new(
                    move |_ctx: &mut runtime::Context,
                          _state: &mut dyn runtime::ReactorState,
                          inputs: &[runtime::IPort],
                          _outputs: &mut [runtime::OPort],
                          _actions: &mut [&mut runtime::Action]| {
                        let [port]: &[runtime::IPort; 1] = inputs.try_into().unwrap();
                        if !port.is_set() {
                            println!("send_port_absent_to_federate({:?})", to_port)
                        }
                    },
                )
            };

            let _ = self
                .add_reaction(
                    &format!("output_control_{from_port_name}_{to_port_name}"),
                    output_control_reaction,
                )
                .with_trigger_action(output_control_trigger, 0)
                .with_trigger_port(*from_port, 0)
                .finish()?;
        }

        Ok(())
    }

    /// For each inward binding on a port in the source `reactor`, create:
    ///  1. a reaction sensitive to a new inputControlReactionTrigger that will call `wait_until_port_status_known`
    ///  2. a reaction sensitive to a new networkMessage action
    pub(crate) fn transform_inward_bindings(
        &mut self,
        inward_bindings: &[(BuilderPortKey, String)],
    ) -> Result<(), BuilderError> {
        for (from_port, from_port_name) in inward_bindings.iter() {
            let input_control_trigger = self
                .add_logical_action::<()>("inputControlReactionTrigger_{from_port_name}", None)?;

            let input_control_reaction = {
                Arc::new(
                    move |_ctx: &mut runtime::Context,
                          _state: &mut dyn runtime::ReactorState,
                          _inputs: &[runtime::IPort],
                          outputs: &mut [runtime::OPort],
                          _actions: &mut [&mut runtime::Action]| {
                        let [port]: &mut [runtime::OPort; 1] = outputs.try_into().unwrap();
                        if port.is_set() {
                            println!("wait_until_port_status_known");
                        }
                    },
                )
            };

            let _ = self
                .add_reaction(
                    &(format!("input_control_{from_port_name}")),
                    input_control_reaction,
                )
                .with_trigger_action(input_control_trigger, 0)
                .with_antidependency(*from_port, 0)
                .finish()?;
        }

        Ok(())
    }
}

/// Transformation methods for a Federated Environment
impl EnvBuilder {
    fn prepare_outward_bindings(
        &self,
        child_reactor_key: BuilderReactorKey,
    ) -> Vec<(BuilderPortKey, String, BuilderPortKey, String)> {
        self.reactor_builders[child_reactor_key]
            .ports
            .keys()
            .filter_map(|port_key| {
                let port = &self.port_builders[port_key];
                if port.get_port_type() == PortType::Output {
                    Some((port_key, port))
                } else {
                    None
                }
            })
            .flat_map(|(port_key, port)| {
                port.get_outward_bindings()
                    .map(move |binding_key| (port_key, binding_key))
            })
            .map(|(from_port, to_port)| {
                let from_port_name = self.port_builders[from_port].get_name();
                let to_port_name = self.port_builders[to_port].get_name();
                (
                    from_port,
                    from_port_name.to_owned(),
                    to_port,
                    to_port_name.to_owned(),
                )
            })
            .collect()
    }

    fn prepare_inward_bindings(
        &self,
        child_reactor_key: BuilderReactorKey,
    ) -> Vec<(BuilderPortKey, String)> {
        self.reactor_builders[child_reactor_key]
            .ports
            .keys()
            .filter(|&port_key| {
                let port = &self.port_builders[port_key];
                port.get_port_type() == PortType::Input && port.get_inward_binding().is_some()
            })
            .map(|binding_key| {
                let port_name = self.port_builders[binding_key].get_name();
                (binding_key, port_name.to_owned())
            })
            .collect()
    }

    /// Clone an existing reactor as a new Federate.
    ///
    /// This clones the parent reactor replacing any port bindings with IO using the federate
    /// infrastructure.
    pub fn clone_reactor_as_federate(
        &mut self,
        child_reactor_key: BuilderReactorKey,
    ) -> Result<BuilderReactorKey, BuilderError> {
        let child_reactor = &self.reactor_builders[child_reactor_key];

        let outward_bindings = self.prepare_outward_bindings(child_reactor_key);
        let inward_bindings = self.prepare_inward_bindings(child_reactor_key);

        // Clear the outward bindings on the ports in the parent reactor.
        for (port_key, _, _, _) in outward_bindings.iter() {
            self.port_builders[*port_key].clear_outward_bindings();
        }

        for (port_key, _) in inward_bindings.iter() {
            self.port_builders[*port_key].clear_inward_binding();
        }

        let parent = child_reactor
            .parent_reactor_key
            .ok_or_else(|| BuilderError::NotChildReactor {
                reactor: child_reactor_key,
            })
            .map(|parent_key| &self.reactor_builders[parent_key])?;

        let federate_name = format!("{}-federate", child_reactor.get_name());

        // These reactions will be cloned to each federate from the original reactor.
        let filtered_reactions = parent.reactions.keys().filter_map(|reaction_key| {
            let reaction = &self.reaction_builders[reaction_key];

            // If the reaction is bound to a port not in the child reactor, we need to drop it and issue a warning.
            if reaction.input_ports.keys().chain(reaction.output_ports.keys()).find(|&port_key| {
                child_reactor.ports.get(port_key).is_none()
            }).is_some() {
                tracing::warn!("Dropping reaction {} while building {federate_name} because it is bound to a port in a different reactor", reaction.name);
                None
            }
            else {
                Some(reaction_key)
            }
        }).collect_vec();

        let mut builder = self.add_reactor(&federate_name, None, parent.state.clone());

        builder.adopt_existing_child(child_reactor_key);
        builder.clone_existing_reactions(filtered_reactions.iter().copied());

        builder.transform_outward_bindings(&outward_bindings)?;
        builder.transform_inward_bindings(&inward_bindings)?;

        let new_reactor_key = builder.finish()?;

        /*
        let new_reactor_key = self
            .reactor_builders
            .insert_with_key(|reactor_key| ReactorBuilder {
                name: federate_name,
                state: parent.state.clone(),
                type_name: parent.type_name.clone(),
                parent_reactor_key: None,
                reactions: filtered_reactions,
                ports: Default::default(),
                actions: parent.actions.clone(),
            });
            */

        Ok(new_reactor_key)
    }

    /// Transform the top-level reactor specified into multiple federated reactors.
    pub fn federalize_reactor(
        &mut self,
        reactor_key: BuilderReactorKey,
    ) -> Result<(), BuilderError> {
        if let Some(parent_reactor_key) = self.reactor_builders[reactor_key].parent_reactor_key {
            return Err(BuilderError::NotTopLevelReactor {
                parent: parent_reactor_key,
            });
        }

        let reactor = &self.reactor_builders[reactor_key];

        // Find all reactors that are children of this reactor.
        let children = self
            .reactor_builders
            .iter()
            .filter_map(|(key, builder)| {
                if builder.parent_reactor_key == Some(reactor_key) {
                    Some(key)
                } else {
                    None
                }
            })
            .collect_vec();

        for child in children {
            self.clone_reactor_as_federate(child)?;
        }

        Ok(())
    }
}
