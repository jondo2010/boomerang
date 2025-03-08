//! Specialized builder for non-port-binding connections between reactors.
//!
//! Non-port-binding connections are connections with a specified delay or between enclaves.

use std::collections::{BTreeSet, HashMap};

use slotmap::SecondaryMap;

use crate::{
    runtime, BuilderActionKey, BuilderError, BuilderPortKey, BuilderReactorKey, EnclaveDep,
    EnvBuilder, Input, Output, ParentReactorBuilder, PartitionMap, PortType, TriggerMode,
    TypedPortKey,
};

#[derive(Default)]
pub struct PortBindings {
    inward: SecondaryMap<BuilderPortKey, BuilderPortKey>,
    outward: SecondaryMap<BuilderPortKey, BTreeSet<BuilderPortKey>>,
}

impl std::fmt::Debug for PortBindings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inward = self
            .inward
            .iter()
            .map(|(k, v)| (format!("{k:?}"), format!("{v:?}")))
            .collect::<HashMap<String, String>>();

        let outward = self
            .outward
            .iter()
            .map(|(k, v)| {
                (
                    format!("{k:?}"),
                    v.iter().map(|k| format!("{k:?}")).collect::<Vec<_>>(),
                )
            })
            .collect::<HashMap<String, Vec<String>>>();

        f.debug_struct("PortBindings")
            .field("inward", &inward)
            .field("outward", &outward)
            .finish()
    }
}

impl PortBindings {
    pub fn bind(
        &mut self,
        source_key: BuilderPortKey,
        target_key: BuilderPortKey,
        env: &EnvBuilder,
    ) -> Result<(), BuilderError> {
        if let Some(existing) = self.inward.get(target_key) {
            return Err(BuilderError::PortConnectionError {
                source_key,
                target_key,
                what: format!(
                    "Ports may only be connected once, but `target` is already connected to {existing:?}",
                ),
            });
        }

        if env.reaction_builders.iter().any(|(_, reaction)| {
            reaction
                .port_relations
                .iter()
                .any(|(port_key, trigger_mode)| match trigger_mode {
                    TriggerMode::TriggersAndUses | TriggerMode::UsesOnly
                        if port_key == source_key =>
                    {
                        true
                    }
                    TriggerMode::EffectsOnly | TriggerMode::TriggersAndEffects
                        if port_key == target_key =>
                    {
                        true
                    }
                    _ => false,
                })
        }) {
            return Err(BuilderError::PortConnectionError {
                source_key,
                target_key,
                what: "Ports with Uses or Effects relations may not be connected to other ports"
                    .to_owned(),
            });
        }

        let source_port = &env.port_builders[source_key];
        let target_port = &env.port_builders[target_key];

        let source_ancestor =
            env.reactor_builders[source_port.get_reactor_key()].parent_reactor_key;
        let target_ancestor =
            env.reactor_builders[target_port.get_reactor_key()].parent_reactor_key;

        match (source_port.port_type(), target_port.port_type()) {
            (PortType::Input, PortType::Input) => {
                match target_ancestor {
                    Some(key) if key == source_port.get_reactor_key() => {
                        // Valid
                    }
                    _ => {
                        return Err(BuilderError::PortConnectionError {
                            source_key,
                            target_key,
                            what: "An input port A may only be bound to another input port B if B is contained by a reactor that in turn is contained by the reactor of A.".into(),
                        });
                    }
                }
            }
            (PortType::Output, PortType::Input) => {
                // VALIDATE(this->container()->container() == port->container()->container(),
                if source_ancestor != target_ancestor {
                    return Err(BuilderError::PortConnectionError {
                        source_key,
                        target_key,
                        what: "An output port A may only be bound to an input port B if both ports belong to reactors in the same hierarichal level.".into(),
                    });
                }

                /*
                match (source_ancestor, target_ancestor) {
                    (Some(key_a), Some(key_b)) if key_a == key_b => {
                        // Valid
                    }
                    _ => {
                        dbg!(source_ancestor, target_ancestor);
                        return Err(BuilderError::PortConnectionError {
                            source_key,
                            target_key,
                            what: "An output port A may only be bound to an input port B if both ports belong to reactors in the same hierarichal level.".into(),
                        });
                    }
                }
                */
            }
            (PortType::Output, PortType::Output) => {
                // VALIDATE( this->container()->container() == port->container(),
                match source_ancestor {
                    Some(key) if key == target_port.get_reactor_key() => {
                        // Valid
                    }
                    _ => {
                        return Err(BuilderError::PortConnectionError {
                            source_key,
                            target_key,
                            what: "An output port A may only be bound to another output port B if A is contained by a reactor that in turn is contained by the reactor of B".into(),
                        });
                    }
                }
            }
            (PortType::Input, PortType::Output) => {
                return Err(BuilderError::PortConnectionError {
                    source_key,
                    target_key,
                    what: "Unexpected case: can't bind an input Port to an output Port.".to_owned(),
                });
            }
        }

        // All validity checks passed, so we can now bind the ports
        self.inward.insert(target_key, source_key);
        self.outward
            .entry(source_key)
            .unwrap()
            .or_default()
            .insert(target_key);

        Ok(())
    }

    /// Follow the inward bindings of a Port to the source
    pub fn follow_port_inward(&self, port_key: BuilderPortKey) -> BuilderPortKey {
        let mut cur_key = port_key;
        while let Some(new_idx) = self.inward.get(cur_key) {
            cur_key = *new_idx;
        }
        cur_key
    }

    /// Get the outward bindings of a Port
    pub fn get_outward_bindings(
        &self,
        port_key: BuilderPortKey,
    ) -> impl Iterator<Item = BuilderPortKey> + use<'_> {
        self.outward
            .get(port_key)
            .into_iter()
            .flat_map(|set| set.iter().cloned())
    }
}

pub trait BaseConnectionBuilder {
    fn source_key(&self) -> BuilderPortKey;
    fn target_key(&self) -> BuilderPortKey;
    fn after(&self) -> Option<runtime::Duration>;
    fn physical(&self) -> bool;
    /// Build the connection between two ports
    fn build(
        &self,
        env: &mut EnvBuilder,
        partition_map: &mut PartitionMap,
        port_bindings: &mut PortBindings,
        enclave_deps: &mut Vec<EnclaveDep>,
    ) -> Result<(), BuilderError>;
}

pub struct ConnectionBuilder<T: runtime::ReactorData> {
    pub(crate) source_key: BuilderPortKey,
    pub(crate) target_key: BuilderPortKey,
    pub(crate) after: Option<runtime::Duration>,
    pub(crate) physical: bool,
    pub(crate) _phantom: std::marker::PhantomData<fn() -> T>,
}

impl<T: runtime::ReactorData + Clone> BaseConnectionBuilder for ConnectionBuilder<T> {
    fn source_key(&self) -> BuilderPortKey {
        self.source_key
    }
    fn target_key(&self) -> BuilderPortKey {
        self.target_key
    }
    fn after(&self) -> Option<runtime::Duration> {
        self.after
    }
    fn physical(&self) -> bool {
        self.physical
    }
    fn build(
        &self,
        env: &mut EnvBuilder,
        partition_map: &mut PartitionMap,
        port_bindings: &mut PortBindings,
        enclave_deps: &mut Vec<EnclaveDep>,
    ) -> Result<(), BuilderError> {
        let source_port = &env.port_builders[self.source_key()];
        let target_port = &env.port_builders[self.target_key()];

        let source_reactor_key = source_port.parent_reactor_key().unwrap();
        let target_reactor_key = target_port.parent_reactor_key().unwrap();

        let source_partition = partition_map[source_reactor_key];
        let target_partition = partition_map[target_reactor_key];

        if source_partition == target_partition {
            // Ports connected with a delay and/or physical connections are implemented as a pair of Reactions that trigger and react to an action.
            if self.physical || self.after.is_some() {
                let source_parent_reactor_key =
                    env.reactor_builders[source_reactor_key].parent_reactor_key();
                let target_parent_reactor_key =
                    env.reactor_builders[target_reactor_key].parent_reactor_key();
                assert_eq!(
                    source_parent_reactor_key, target_parent_reactor_key,
                    "Delayed connections between same ancestor?"
                );
                let (reactor_key, input_port, output_port) = build_delayed_connection::<T>(
                    env,
                    source_parent_reactor_key,
                    self.physical,
                    self.after,
                )?;
                partition_map.insert(reactor_key, source_partition);
                port_bindings.bind(self.source_key, input_port.into(), env)?;
                port_bindings.bind(output_port.into(), self.target_key, env)?;
            } else {
                // Simple case, we can just bind them directly
                port_bindings.bind(self.source_key, self.target_key, env)?;
            }
        } else {
            // The connection is between two different partitions, so we need to build a pair of Reactions that trigger and react to an Action.
            let target_parent_reactor_key =
                env.reactor_builders[target_reactor_key].parent_reactor_key();
            let (target_reactor_key, output_port, target_action_key) =
                build_enclave_connection_target::<T>(
                    env,
                    target_parent_reactor_key,
                    self.physical,
                    self.after,
                )?;
            partition_map.insert(target_reactor_key, target_partition);
            port_bindings.bind(output_port.into(), self.target_key, env)?;

            let source_parent_reactor_key =
                env.reactor_builders[source_reactor_key].parent_reactor_key();
            let (source_reactor_key, input_port) = build_enclave_connection_source::<T>(
                env,
                source_parent_reactor_key,
                target_partition,
                target_action_key,
            )?;
            partition_map.insert(source_reactor_key, source_partition);
            port_bindings.bind(self.source_key, input_port.into(), env)?;

            enclave_deps.push(EnclaveDep {
                upstream: source_partition,
                downstream: target_partition,
                delay: self.after,
            });
        }
        Ok(())
    }
}

/// Build a delayed or physical connection between two ports.
///
/// The connection is built as a pair of Reactions that trigger and react to an Action.
#[allow(clippy::type_complexity)]
fn build_delayed_connection<T: runtime::ReactorData + Clone>(
    env: &mut EnvBuilder,
    parent_key: Option<BuilderReactorKey>,
    physical: bool,
    after: Option<runtime::Duration>,
) -> Result<
    (
        BuilderReactorKey,
        TypedPortKey<T, Input>,
        TypedPortKey<T, Output>,
    ),
    BuilderError,
> {
    let mut builder = env.add_reactor("con_reactor", parent_key, None, (), false);
    let input_port = builder.add_input_port::<T>("con_in")?;
    let output_port = builder.add_output_port::<T>("con_out")?;
    let action_key: BuilderActionKey = if physical {
        builder.add_physical_action::<T>("con_act", after)?.into()
    } else {
        builder.add_logical_action::<T>("con_act", after)?.into()
    };
    // The target reaction is triggered by the action, and writes to the output port
    let _ = builder
        .add_reaction("con_tgt", |_| {
            runtime::ConnectionReceiverReactionFn::<T>::default().into()
        })
        .with_action(action_key, 0, TriggerMode::TriggersAndUses)?
        .with_port(output_port, 0, TriggerMode::EffectsOnly)?
        .finish()?;
    // The source reaction is triggered by the input port, and schedules the action
    let _ = builder
        .add_reaction("con_src", |_| {
            runtime::ConnectionSenderReactionFn::<T>::default().into()
        })
        .with_action(action_key, 0, TriggerMode::EffectsOnly)?
        .with_port(input_port, 0, TriggerMode::TriggersAndUses)?
        .finish()?;
    let reactor_key = builder.finish()?;
    Ok((reactor_key, input_port, output_port))
}

/// Build the source portion
///
/// The sender-side is build-deferred by returning a closure. The BuilderAction must be turned into a runtime Action before the closure is called.
fn build_enclave_connection_source<T: runtime::ReactorData + Clone>(
    env: &mut EnvBuilder,
    parent_key: Option<BuilderReactorKey>,
    target_partition: BuilderReactorKey,
    target_action_key: BuilderActionKey,
) -> Result<(BuilderReactorKey, TypedPortKey<T, Input>), BuilderError> {
    let mut source_builder = env.add_reactor("con_reactor_src", parent_key, None, (), false);
    let input_port = source_builder.add_input_port::<T>("con_in")?;
    let _ = source_builder
        .add_reaction("con_react_src", move |builder_parts| {
            let (enclave_key, runtime_action_key) = builder_parts
                .aliases
                .action_aliases
                .get(target_action_key)
                .expect("Action key");
            let enclave = &builder_parts.enclaves[*enclave_key];

            //TODO: Get rid of this and the target_partition argument once this works
            let enclave_key2 = builder_parts.aliases.enclave_aliases[target_partition];
            assert_eq!(enclave_key, &enclave_key2, "Temporary cross-check");

            let remote_context = enclave.create_send_context(*enclave_key);
            let remote_action_ref = enclave.create_async_action_ref(*runtime_action_key);
            runtime::EnclaveSenderReactionFn::<T>::new(
                remote_context,
                remote_action_ref,
                None,
                //Some(runtime::Duration::milliseconds(500)),
            )
            .into()
        })
        .with_port(input_port, 0, TriggerMode::TriggersAndUses)?
        .finish()?;

    let reactor_key = source_builder.finish()?;
    Ok((reactor_key, input_port))
}

/// Build the target portion
///
/// The receiver-side of is built immediately into the `EnvBuilder`, and consists of an Action that triggers a Reaction that writes to the target port.
fn build_enclave_connection_target<T: runtime::ReactorData + Clone>(
    env: &mut EnvBuilder,
    parent_key: Option<BuilderReactorKey>,
    physical: bool,
    after: Option<runtime::Duration>,
) -> Result<(BuilderReactorKey, TypedPortKey<T, Output>, BuilderActionKey), BuilderError> {
    let mut target_builder = env.add_reactor("con_reactor_tgt", parent_key, None, (), false);
    let output_port = target_builder.add_output_port::<T>("con_out")?;
    let action_key: BuilderActionKey = if physical {
        target_builder
            .add_physical_action::<T>("con_act", after)?
            .into()
    } else {
        target_builder
            .add_logical_action::<T>("con_act", after)?
            .into()
    };
    let _ = target_builder
        .add_reaction("con_react_tgt", |_| {
            runtime::ConnectionReceiverReactionFn::<T>::default().into()
        })
        .with_action(action_key, 0, TriggerMode::TriggersAndUses)?
        .with_port(output_port, 0, TriggerMode::EffectsOnly)?
        .finish()?;
    let reactor_key = target_builder.finish()?;
    Ok((reactor_key, output_port, action_key))
}
