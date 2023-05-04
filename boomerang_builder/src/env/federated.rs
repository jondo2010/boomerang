//! Implementation of the `EnvBuilder` trait for federated environments.

use std::{sync::Arc, time::Duration};

use boomerang_federated as federated;
use boomerang_runtime as runtime;

use federated::FederateKey;
use itertools::Itertools;
use slotmap::SecondaryMap;

use super::{output::RuntimePortParts, EnvBuilder};
use crate::{BuilderActionKey, BuilderError, BuilderPortKey, BuilderReactorKey, PortType};

pub struct Bindings {
    pub inward: Vec<(BuilderPortKey, BuilderPortKey)>,
    pub outward: Vec<(BuilderPortKey, BuilderPortKey)>,
}

struct InwardControlAction {
    input_control_triggers: Vec<BuilderActionKey>,
    network_messages: Vec<BuilderActionKey>,
}

struct OutwardControlAction {
    output_control_trigger: BuilderActionKey,
}

/// Transformation methods for a Federated Environment
impl EnvBuilder {
    /// For each inward binding on a port in the reactor, create:
    ///  1. a reaction sensitive to a new inputControlReactionTrigger that will call `wait_until_port_status_known`
    ///  2. a reaction sensitive to a new networkMessage action
    fn transform_inward_bindings(
        &mut self,
        reactor_key: BuilderReactorKey,
        inward_bindings: &[(BuilderPortKey, BuilderPortKey)],
        runtime_port_aliases: &SecondaryMap<BuilderPortKey, runtime::keys::PortKey>,
        _bound_ports_map: &SecondaryMap<BuilderPortKey, FederateKey>,
    ) -> Result<InwardControlAction, BuilderError> {
        let control_actions = inward_bindings.iter().map(|(own_port, _foreign_port)| {
            let own_port_name = self.port_builders[*own_port].get_name().to_owned();
            let runtime_port_key = runtime_port_aliases[*own_port];

            let input_control_trigger: BuilderActionKey = {
                let input_control_trigger = self.add_logical_action::<()>(
                    &format!("in_ctrl_trigger_{own_port_name}"),
                    None,
                    reactor_key,
                )?;
                let input_control_reaction = Arc::new(
                    move |_ctx: &mut runtime::Context,
                          _state: &mut dyn runtime::ReactorState,
                          _inputs: &[runtime::IPort],
                          outputs: &mut [runtime::OPort],
                          _actions: &mut [&mut runtime::Action]| {
                        let [port]: &mut [runtime::OPort; 1] = outputs.try_into().unwrap();
                        if port.is_set() {
                            println!("wait_until_port_status_known({:?})", runtime_port_key);
                        }
                    },
                );

                let _ = self
                    .add_reaction(
                        &format!("in_ctrl_{own_port_name}"),
                        reactor_key,
                        input_control_reaction,
                    )
                    .with_trigger_action(input_control_trigger, 0)
                    .with_antidependency(*own_port, 0)
                    .finish()?;

                input_control_trigger.into()
            };

            let network_message_trigger = {
                let network_message_trigger = self.add_logical_action_from_port(
                    &format!("network_message_{own_port_name}"),
                    *own_port,
                    reactor_key,
                )?;

                let network_reaction = Arc::new(
                    move |_ctx: &mut runtime::Context,
                          _state: &mut dyn runtime::ReactorState,
                          _inputs: &[runtime::IPort],
                          outputs: &mut [runtime::OPort],
                          _actions: &mut [&mut runtime::Action]| {
                        let [port]: &mut [runtime::OPort; 1] = outputs.try_into().unwrap();
                        if port.is_set() {
                            println!("set_port_value({:?})", runtime_port_key);
                        }
                    },
                );

                let _ = self
                    .add_reaction(
                        &format!("reaction_network_{own_port_name}"),
                        reactor_key,
                        network_reaction,
                    )
                    .with_trigger_action(network_message_trigger, 0)
                    .with_antidependency(*own_port, 0)
                    .finish()?;
                network_message_trigger
            };

            Ok::<_, BuilderError>((input_control_trigger, network_message_trigger))
        });

        let (input_control_triggers, network_messages) =
            itertools::process_results(control_actions, |x| x.unzip())?;

        Ok(InwardControlAction {
            input_control_triggers,
            network_messages,
        })
    }

    /// For each outward binding on a port in reactor, create:
    /// 1. a reaction to forward it with `send_timed_message`, and a
    /// 2. a reaction sensitive to the outputControlReactonTrigger that will call `send_port_absent` if the port isn't set.
    fn transform_outward_bindings(
        &mut self,
        reactor_key: BuilderReactorKey,
        outward_bindings: &[(BuilderPortKey, BuilderPortKey)],
        runtime_port_aliases: &SecondaryMap<BuilderPortKey, runtime::keys::PortKey>,
        bound_ports_map: &SecondaryMap<BuilderPortKey, FederateKey>,
    ) -> Result<OutwardControlAction, BuilderError> {
        let output_control_trigger =
            self.add_logical_action::<()>("out_ctrl_trigger", None, reactor_key)?;

        for (own_port, foreign_port) in outward_bindings.iter() {
            let own_port_name = self.port_builders[*own_port].get_name().to_owned();
            let to_port_name = self.port_builders[*foreign_port].get_name().to_owned();
            let runtime_to_port = runtime_port_aliases[*foreign_port];
            let foreign_federate = bound_ports_map[*foreign_port];

            {
                let output_binding_reaction = Arc::new(
                    move |ctx: &mut runtime::Context,
                          _state: &mut dyn runtime::ReactorState,
                          inputs: &[runtime::IPort],
                          _outputs: &mut [runtime::OPort],
                          _actions: &mut [&mut runtime::Action]| {
                        let [port]: [runtime::IPort; 1] = inputs.try_into().unwrap();
                        if port.is_set() {
                            ctx.send_timed_message(foreign_federate, runtime_to_port, None, ())
                                .unwrap();
                        }
                    },
                );

                let _ = self
                    .add_reaction(
                        &(format!("{own_port_name}_{to_port_name}")),
                        reactor_key,
                        output_binding_reaction,
                    )
                    .with_trigger_port(*own_port, 0)
                    .finish()?;
            }

            {
                let output_control_reaction = Arc::new(
                    move |ctx: &mut runtime::Context,
                          _state: &mut dyn runtime::ReactorState,
                          inputs: &[runtime::IPort],
                          _outputs: &mut [runtime::OPort],
                          _actions: &mut [&mut runtime::Action]| {
                        let [port]: &[runtime::IPort; 1] = inputs.try_into().unwrap();
                        if !port.is_set() {
                            ctx.send_port_absent_to_federate(
                                foreign_federate,
                                runtime_to_port,
                                None,
                            )
                            .unwrap();
                        }
                    },
                );

                let _ = self
                    .add_reaction(
                        &format!("out_ctrl_{own_port_name}_{to_port_name}"),
                        reactor_key,
                        output_control_reaction,
                    )
                    .with_trigger_action(output_control_trigger, 0)
                    .with_trigger_port(*own_port, 0)
                    .finish()?;
            }
        }

        Ok(OutwardControlAction {
            output_control_trigger: output_control_trigger.into(),
        })
    }

    fn prepare_outward_bindings(
        &self,
        child_reactor_key: BuilderReactorKey,
    ) -> Vec<(BuilderPortKey, BuilderPortKey)> {
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
            .collect()
    }

    fn prepare_inward_bindings(
        &self,
        child_reactor_key: BuilderReactorKey,
    ) -> Vec<(BuilderPortKey, BuilderPortKey)> {
        self.reactor_builders[child_reactor_key]
            .ports
            .keys()
            .filter_map(|port_key| {
                let port = &self.port_builders[port_key];
                port.get_inward_binding().and_then(|binding_key| {
                    (port.get_port_type() == PortType::Input).then(|| (port_key, binding_key))
                })
            })
            .collect()
    }

    /// Extract (and clear) the inward and outward bindings for a federate.
    pub fn extract_bindings_for_federate(
        &mut self,
        child_reactor_key: BuilderReactorKey,
    ) -> Bindings {
        let inward_bindings = self.prepare_inward_bindings(child_reactor_key);
        let outward_bindings = self.prepare_outward_bindings(child_reactor_key);

        // Clear the outward bindings on the ports in the parent reactor.
        for (port_key, _) in outward_bindings.iter() {
            self.port_builders[*port_key].clear_outward_bindings();
        }

        for (port_key, _) in inward_bindings.iter() {
            self.port_builders[*port_key].clear_inward_binding();
        }

        Bindings {
            inward: inward_bindings,
            outward: outward_bindings,
        }
    }

    /// Clone an existing reactor as a new Federate.
    ///
    /// This clones the parent reactor replacing any port bindings with IO using the federate
    /// infrastructure.
    ///
    /// Returns the key of the new reactor, the inward bindings and the outward bindings.
    pub fn clone_parent_reactor_as_federate(
        &mut self,
        child_reactor_key: BuilderReactorKey,
    ) -> Result<BuilderReactorKey, BuilderError> {
        let child_reactor = &self.reactor_builders[child_reactor_key];

        let parent_reactor_key =
            child_reactor
                .parent_reactor_key
                .ok_or_else(|| BuilderError::NotChildReactor {
                    reactor: child_reactor_key,
                })?;

        let federate_name = format!("{}-federate", child_reactor.get_name());

        // These reactions will be cloned to each federate from the original reactor.
        let filtered_reactions = self.reactor_builders[parent_reactor_key].reactions.keys().filter_map(|reaction_key| {
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

        let mut builder = self.add_reactor_cloned(&federate_name, None, parent_reactor_key);
        let action_mapping = builder.clone_reactor_actions(parent_reactor_key);

        builder.adopt_existing_child(child_reactor_key);
        builder.clone_existing_reactions(filtered_reactions.iter().copied(), &action_mapping);

        Ok(builder.finish()?)
    }

    /// Create the federate reactions for each child using the inward and outward bindings and the runtime ports.
    ///
    /// `new_parents_map` is a map from the new federate keys to the parent reactor key and the inward and outward bindings.
    fn build_federate_runtimes(
        &mut self,
        new_parents_map: &tinymap::TinyMap<FederateKey, (BuilderReactorKey, Bindings)>,
        runtime_ports: &RuntimePortParts,
    ) -> Result<
        tinymap::TinySecondaryMap<FederateKey, (runtime::Env, runtime::FederateEnv)>,
        BuilderError,
    > {
        // Create a mapping between binding ports and the federate they are contained in.
        let bound_ports_map: SecondaryMap<BuilderPortKey, FederateKey> = new_parents_map
            .iter()
            .flat_map(|(federate_key, (_, bindings))| {
                let inward = bindings
                    .inward
                    .iter()
                    .map(move |(port_key, _)| (*port_key, federate_key));
                let outward = bindings
                    .outward
                    .iter()
                    .map(move |(port_key, _)| (*port_key, federate_key));
                inward.chain(outward)
            })
            .collect();

        let federates = new_parents_map
            .iter()
            .map(|(federate_key, (reactor_key, bindings))| {
                let inward_control_actions = self.transform_inward_bindings(
                    *reactor_key,
                    &bindings.inward,
                    &runtime_ports.aliases,
                    &bound_ports_map,
                )?;
                let outward_control_action = self.transform_outward_bindings(
                    *reactor_key,
                    &bindings.outward,
                    &runtime_ports.aliases,
                    &bound_ports_map,
                )?;

                let (env, aliases) = self.build_runtime(*reactor_key)?;

                // Un-alias the control actions so we can return runtime keys.
                let input_control_triggers = inward_control_actions
                    .input_control_triggers
                    .into_iter()
                    .map(|builder_action_key| {
                        aliases.action_aliases[*reactor_key][builder_action_key]
                    })
                    .collect_vec();
                let network_messages = inward_control_actions
                    .network_messages
                    .into_iter()
                    .map(|builder_action_key| {
                        aliases.action_aliases[*reactor_key][builder_action_key]
                    })
                    .collect_vec();
                let output_control_trigger = aliases.action_aliases[*reactor_key]
                    [outward_control_action.output_control_trigger];

                let neighbors = federated::NeighborStructure {
                    upstream: bindings
                        .inward
                        .iter()
                        .map(|(_, port_key)| (bound_ports_map[*port_key], Duration::ZERO))
                        .collect(),
                    downstream: bindings
                        .outward
                        .iter()
                        .map(|(_, port_key)| bound_ports_map[*port_key])
                        .collect(),
                };

                Ok((
                    federate_key,
                    (
                        env,
                        runtime::FederateEnv {
                            input_control_triggers,
                            network_messages,
                            output_control_trigger,
                            neighbors,
                        },
                    ),
                ))
            })
            .collect::<Result<_, BuilderError>>()?;

        Ok(federates)
    }

    /// Transform the top-level reactor specified into multiple federated reactors.
    ///
    /// A map of [`FederateKey`] -> [`FederateEnv`] is returned.
    pub fn federalize_reactor(
        &mut self,
        reactor_key: BuilderReactorKey,
    ) -> Result<
        tinymap::TinySecondaryMap<FederateKey, (runtime::Env, runtime::FederateEnv)>,
        BuilderError,
    > {
        if let Some(parent_reactor_key) = self.reactor_builders[reactor_key].parent_reactor_key {
            return Err(BuilderError::NotTopLevelReactor {
                parent: parent_reactor_key,
            });
        }

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

        tracing::info!(
            "Preparing reactor {} for distributed execution with {} nodes.",
            self.reactor_builders[reactor_key].get_name(),
            children.len()
        );

        // Build the runtime ports for all reactors before any transformations.
        let runtime_ports = self.build_runtime_ports(self.reactor_builders.keys());

        // Create a new top-level federate reactor for each child.
        // This creates a map of (parent_reactor_key, child_reactor_key)
        let new_parents_map: tinymap::TinyMap<FederateKey, (BuilderReactorKey, Bindings)> =
            children
                .iter()
                .map(|&child_key| {
                    // Extract the inward and outward bindings for each child.
                    let bindings = self.extract_bindings_for_federate(child_key);
                    // Clone the parent reactor as a new federate parent for the child.
                    let new_parent_key = self.clone_parent_reactor_as_federate(child_key)?;
                    Ok((new_parent_key, bindings))
                })
                .collect::<Result<_, BuilderError>>()?;

        // Remove the original reactor.
        self.remove_reactor(reactor_key)?;

        Ok(self.build_federate_runtimes(&new_parents_map, &runtime_ports)?)
    }
}

#[test]
fn test() {
    use super::tests::test_reactor::*;
    use crate::Reactor;

    let mut env_builder = EnvBuilder::new();
    let (c_key, _) = CBuilder::build("c", (), None, &mut env_builder).unwrap();

    let federates = env_builder.federalize_reactor(c_key).unwrap();
    assert_eq!(federates.len(), 2);

    // Check that the federates are connected correctly. The federate for `a` should have no upstream
    // connections and one downstream connection to `b`. The federate for `b` should have one upstream
    // connection from `a` and no downstream connections.
    let (f0, f1) = federates.keys().collect_tuple().unwrap();
    let neighbors = &federates[f0].1.neighbors;
    assert_eq!(neighbors.upstream, vec![]);
    assert_eq!(neighbors.downstream, vec![f1]);
    assert_eq!(neighbors.upstream, vec![(f0, Duration::ZERO)]);
    assert_eq!(neighbors.downstream, vec![]);
}
