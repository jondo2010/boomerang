//! Specialized assembly support for non-port-binding connections between reactors.
//!
//! Non-port-binding connections are connections with a specified delay or between enclaves.

use std::collections::{BTreeSet, HashMap};
#[cfg(feature = "federated")]
use std::sync::Arc;

use slotmap::SecondaryMap;

use crate::{
    runtime, ActionTag, Assembly, AssemblyActionKey, AssemblyError, AssemblyModeKey,
    AssemblyPortKey, AssemblyReactorKey, Input, Output, ParentReactorSpec, PartitionMap, PortType,
    TriggerMode, TypedActionKey, TypedPortKey,
};

#[cfg(feature = "federated")]
pub(crate) struct FederatedEncoderAdapter<C> {
    pub(crate) codec: Arc<C>,
}

#[cfg(feature = "federated")]
impl<T, C> runtime::FederatedPayloadEncoder<T> for FederatedEncoderAdapter<C>
where
    T: runtime::ReactorData,
    C: boomerang_federated::PayloadEncoder<T> + Send + Sync + 'static,
{
    fn encode(&self, value: &T) -> Result<Vec<u8>, runtime::FederatedEndpointError> {
        self.codec
            .encode(value)
            .map_err(|error| runtime::FederatedEndpointError::codec(error.to_string()))
    }
}

#[cfg(feature = "federated")]
pub(crate) struct FederatedDecoderAdapter<C> {
    pub(crate) codec: Arc<C>,
}

#[cfg(feature = "federated")]
impl<T, C> runtime::FederatedPayloadDecoder<T> for FederatedDecoderAdapter<C>
where
    T: runtime::ReactorData,
    C: boomerang_federated::PayloadDecoder<T> + Send + Sync + 'static,
{
    fn decode(&self, bytes: &[u8]) -> Result<T, runtime::FederatedEndpointError> {
        self.codec
            .decode(bytes)
            .map_err(|error| runtime::FederatedEndpointError::codec(error.to_string()))
    }
}

#[derive(Default)]
pub struct PortBindings {
    inward: SecondaryMap<AssemblyPortKey, AssemblyPortKey>,
    outward: SecondaryMap<AssemblyPortKey, BTreeSet<AssemblyPortKey>>,
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
        source_key: AssemblyPortKey,
        target_key: AssemblyPortKey,
        assembly: &Assembly,
    ) -> Result<(), AssemblyError> {
        if let Some(existing) = self.inward.get(target_key) {
            return Err(AssemblyError::PortConnectionError {
                source_key,
                target_key,
                what: format!(
                    "Ports may only be connected once, but `target` is already connected to {existing:?}",
                ),
            });
        }

        if assembly.reaction_specs.iter().any(|(_, reaction)| {
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
            return Err(AssemblyError::PortConnectionError {
                source_key,
                target_key,
                what: "Ports with Uses or Effects relations may not be connected to other ports"
                    .to_owned(),
            });
        }

        let source_port = &assembly.port_specs[source_key];
        let target_port = &assembly.port_specs[target_key];

        let source_ancestor =
            assembly.reactor_specs[source_port.get_reactor_key()].parent_reactor_key;
        let target_ancestor =
            assembly.reactor_specs[target_port.get_reactor_key()].parent_reactor_key;

        match (source_port.port_type(), target_port.port_type()) {
            (PortType::Input, PortType::Input) => {
                match target_ancestor {
                    Some(key) if key == source_port.get_reactor_key() => {
                        // Valid
                    }
                    _ => {
                        return Err(AssemblyError::PortConnectionError {
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
                    return Err(AssemblyError::PortConnectionError {
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
                        return Err(AssemblyError::PortConnectionError {
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
                        return Err(AssemblyError::PortConnectionError {
                            source_key,
                            target_key,
                            what: "An output port A may only be bound to another output port B if A is contained by a reactor that in turn is contained by the reactor of B".into(),
                        });
                    }
                }
            }
            (PortType::Input, PortType::Output) => {
                return Err(AssemblyError::PortConnectionError {
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
    pub fn follow_port_inward(&self, port_key: AssemblyPortKey) -> AssemblyPortKey {
        let mut cur_key = port_key;
        while let Some(new_idx) = self.inward.get(cur_key) {
            cur_key = *new_idx;
        }
        cur_key
    }

    /// Get the outward bindings of a Port
    pub fn get_outward_bindings(
        &self,
        port_key: AssemblyPortKey,
    ) -> impl Iterator<Item = AssemblyPortKey> + use<'_> {
        self.outward
            .get(port_key)
            .into_iter()
            .flat_map(|set| set.iter().cloned())
    }
}

pub trait ErasedConnectionSpec {
    fn source_key(&self) -> AssemblyPortKey;
    fn target_key(&self) -> AssemblyPortKey;
    fn after(&self) -> Option<runtime::Duration>;
    fn physical(&self) -> bool;
    /// Build the connection between two ports
    fn build(
        &mut self,
        assembly: &mut Assembly,
        partition_map: &mut PartitionMap,
        port_bindings: &mut PortBindings,
    ) -> Result<(), AssemblyError>;
}

pub struct ConnectionSpec<T: runtime::ReactorData, Q: ActionTag> {
    pub(crate) source_key: AssemblyPortKey,
    pub(crate) target_key: AssemblyPortKey,
    pub(crate) after: Option<runtime::Duration>,
    pub(crate) scope_mode: Option<AssemblyModeKey>,
    pub(crate) _phantom: std::marker::PhantomData<fn() -> (T, Q)>,
}

impl<T: runtime::ReactorData + Clone, Q: ActionTag> ErasedConnectionSpec for ConnectionSpec<T, Q> {
    fn source_key(&self) -> AssemblyPortKey {
        self.source_key
    }
    fn target_key(&self) -> AssemblyPortKey {
        self.target_key
    }
    fn after(&self) -> Option<runtime::Duration> {
        self.after
    }
    fn physical(&self) -> bool {
        !Q::IS_LOGICAL
    }
    fn build(
        &mut self,
        assembly: &mut Assembly,
        partition_map: &mut PartitionMap,
        port_bindings: &mut PortBindings,
    ) -> Result<(), AssemblyError> {
        let source_port = &assembly.port_specs[self.source_key()];
        let target_port = &assembly.port_specs[self.target_key()];

        let source_reactor_key = source_port.parent_reactor_key().unwrap();
        let target_reactor_key = target_port.parent_reactor_key().unwrap();

        let source_partition = partition_map[source_reactor_key];
        let target_partition = partition_map[target_reactor_key];

        if source_partition == target_partition {
            // Ports connected with a delay and/or physical connections are implemented as a pair of Reactions that
            // trigger and react to an action.
            if !Q::IS_LOGICAL || self.after.is_some() {
                let source_parent_reactor_key =
                    assembly.reactor_specs[source_reactor_key].parent_reactor_key();
                let target_parent_reactor_key =
                    assembly.reactor_specs[target_reactor_key].parent_reactor_key();
                assert_eq!(
                    source_parent_reactor_key, target_parent_reactor_key,
                    "Delayed connections between same ancestor?"
                );
                let (reactor_key, input_port, output_port) = build_delayed_connection::<T, Q>(
                    assembly,
                    source_parent_reactor_key,
                    self.scope_mode,
                    self.after,
                )?;
                partition_map.insert(reactor_key, source_partition);
                port_bindings.bind(self.source_key, input_port.into(), assembly)?;
                port_bindings.bind(output_port.into(), self.target_key, assembly)?;
            } else {
                // Simple case, we can just bind them directly
                port_bindings.bind(self.source_key, self.target_key, assembly)?;
            }
        } else {
            #[cfg(feature = "federated")]
            if partitions_are_both_federated(assembly, source_partition, target_partition)? {
                if !Q::IS_LOGICAL {
                    return Err(AssemblyError::UnsupportedFederationTopology {
                        what: format!(
                            "cross-federate physical connection '{}' -> '{}' is reserved for a later milestone",
                            assembly.fqn_for(self.source_key, false)?,
                            assembly.fqn_for(self.target_key, false)?,
                        ),
                    });
                }

                let (encoder, decoder) =
                    assembly.federated_codec_for::<T>(self.source_key, self.target_key)?;
                let endpoint = federated_endpoint_id(assembly, self.source_key, self.target_key)?;

                let target_parent_reactor_key =
                    assembly.reactor_specs[target_reactor_key].parent_reactor_key();

                let EnclaveConnectionTarget {
                    reactor_key,
                    output_port,
                    action_key,
                } = build_enclave_connection_target::<T, Q>(
                    assembly,
                    target_parent_reactor_key,
                    self.scope_mode,
                    self.after,
                )?;
                partition_map.insert(reactor_key, target_partition);
                port_bindings.bind(output_port.into(), self.target_key, assembly)?;

                assembly.add_federated_inbound_endpoint::<T>(
                    endpoint.clone(),
                    target_partition,
                    action_key.into(),
                    decoder,
                );

                let source_parent_reactor_key =
                    assembly.reactor_specs[source_reactor_key].parent_reactor_key();
                let EnclaveConnectionSource {
                    reactor_key,
                    input_port,
                } = build_federated_connection_source::<T>(
                    assembly,
                    source_parent_reactor_key,
                    self.scope_mode,
                    target_partition,
                    action_key.into(),
                    endpoint,
                    encoder,
                )?;
                partition_map.insert(reactor_key, source_partition);
                port_bindings.bind(self.source_key, input_port.into(), assembly)?;

                return Ok(());
            }

            // The connection is between two different partitions, so we need to build a pair of Reactions that trigger
            // and react to an Action.
            let target_parent_reactor_key =
                assembly.reactor_specs[target_reactor_key].parent_reactor_key();

            let EnclaveConnectionTarget {
                reactor_key,
                output_port,
                action_key,
            } = build_enclave_connection_target::<T, Q>(
                assembly,
                target_parent_reactor_key,
                self.scope_mode,
                self.after,
            )?;
            partition_map.insert(reactor_key, target_partition);
            port_bindings.bind(output_port.into(), self.target_key, assembly)?;

            let source_parent_reactor_key =
                assembly.reactor_specs[source_reactor_key].parent_reactor_key();
            let EnclaveConnectionSource {
                reactor_key,
                input_port,
            } = build_enclave_connection_source::<T>(
                assembly,
                source_parent_reactor_key,
                self.scope_mode,
                target_partition,
                action_key.into(),
            )?;
            partition_map.insert(reactor_key, source_partition);
            port_bindings.bind(self.source_key, input_port.into(), assembly)?;
        }
        Ok(())
    }
}

#[cfg(feature = "federated")]
fn partitions_are_both_federated(
    assembly: &Assembly,
    source_partition: AssemblyReactorKey,
    target_partition: AssemblyReactorKey,
) -> Result<bool, AssemblyError> {
    let source_federate = assembly.reactor_specs[source_partition].federate_spec();
    let target_federate = assembly.reactor_specs[target_partition].federate_spec();

    match (source_federate.is_some(), target_federate.is_some()) {
        (true, true) => Ok(true),
        (false, false) => Ok(false),
        _ => Err(AssemblyError::UnsupportedFederationTopology {
            what:
                "connection crosses a federated boundary, but both enclave roots are not federates"
                    .to_owned(),
        }),
    }
}

#[cfg(feature = "federated")]
fn federated_endpoint_id(
    assembly: &Assembly,
    source_key: AssemblyPortKey,
    target_key: AssemblyPortKey,
) -> Result<boomerang_federated::EndpointId, AssemblyError> {
    let source_port_fqn = assembly.fqn_for(source_key, false)?.to_string();
    let target_port_fqn = assembly.fqn_for(target_key, false)?.to_string();
    Ok(boomerang_federated::EndpointId::new(format!(
        "{source_port_fqn}->{target_port_fqn}"
    )))
}

/// Build a delayed or physical connection between two ports.
///
/// The connection is built as a pair of Reactions that trigger and react to an Action.
#[allow(clippy::type_complexity)]
fn build_delayed_connection<T: runtime::ReactorData + Clone, Q: ActionTag>(
    assembly: &mut Assembly,
    parent_key: Option<AssemblyReactorKey>,
    scope_mode: Option<AssemblyModeKey>,
    after: Option<runtime::Duration>,
) -> Result<
    (
        AssemblyReactorKey,
        TypedPortKey<T, Input>,
        TypedPortKey<T, Output>,
    ),
    AssemblyError,
> {
    let mut ctx = assembly.add_reactor("con_reactor", parent_key, None, (), false);
    if let Some(scope_mode) = scope_mode {
        ctx.set_scope_mode(scope_mode)?;
    }
    let input_port = ctx.add_input_port::<T>("con_in")?;
    let output_port = ctx.add_output_port::<T>("con_out")?;
    let action_key = ctx.add_action::<T, Q>("con_act", after)?;
    // The target reaction is triggered by the action, and writes to the output port
    let _ = ctx
        .add_reaction(None)
        .with_trigger(action_key)
        .with_effect(output_port)
        .with_deferred_reaction_factory(move |_| {
            runtime::ConnectionReceiverReactionFn::<T>::default().into()
        })
        .finish()?;
    // The source reaction is triggered by the input port, and schedules the action
    let _ = ctx
        .add_reaction(None)
        .with_effect(action_key)
        .with_trigger(input_port)
        .with_deferred_reaction_factory(move |_| {
            runtime::ConnectionSenderReactionFn::<T>::default().into()
        })
        .finish()?;
    let reactor_key = ctx.finish()?;
    Ok((reactor_key, input_port, output_port))
}

struct EnclaveConnectionSource<T: runtime::ReactorData + Clone> {
    reactor_key: AssemblyReactorKey,
    input_port: TypedPortKey<T, Input>,
}

/// Build the source portion
///
/// The sender side is deferred until its assembly action can be resolved to a runtime action.
fn build_enclave_connection_source<T: runtime::ReactorData + Clone>(
    assembly: &mut Assembly,
    parent_key: Option<AssemblyReactorKey>,
    scope_mode: Option<AssemblyModeKey>,
    target_partition: AssemblyReactorKey,
    target_action_key: AssemblyActionKey,
) -> Result<EnclaveConnectionSource<T>, AssemblyError> {
    let mut source_ctx = assembly.add_reactor("con_reactor_src", parent_key, None, (), false);
    if let Some(scope_mode) = scope_mode {
        source_ctx.set_scope_mode(scope_mode)?;
    }
    let input_port = source_ctx.add_input_port::<T>("con_in")?;
    source_ctx
        .add_reaction(None)
        .with_trigger(input_port)
        .with_deferred_reaction_factory(move |runtime_assembly| {
            let (enclave_key, runtime_action_key) = runtime_assembly
                .aliases
                .action_aliases
                .get(target_action_key)
                .expect("Action key");
            let enclave = &runtime_assembly.enclaves[*enclave_key];

            //TODO: Get rid of this and the target_partition argument once this works
            let enclave_key2 = runtime_assembly.aliases.enclave_aliases[target_partition];
            assert_eq!(enclave_key, &enclave_key2, "Temporary cross-check");

            let remote_context = enclave.create_send_context(*enclave_key);
            let remote_action_ref = enclave.create_async_action_ref(*runtime_action_key);
            runtime::InterPartitionSenderReactionFn::<T>::new(
                remote_action_ref,
                Box::new(runtime::InProcessInterPartitionEventSink::new(
                    remote_context,
                )),
                None,
            )
            .into()
        })
        .finish()?;
    let reactor_key = source_ctx.finish()?;
    Ok(EnclaveConnectionSource {
        reactor_key,
        input_port,
    })
}

#[cfg(feature = "federated")]
fn build_federated_connection_source<T: runtime::ReactorData + Clone>(
    assembly: &mut Assembly,
    parent_key: Option<AssemblyReactorKey>,
    scope_mode: Option<AssemblyModeKey>,
    target_partition: AssemblyReactorKey,
    target_action_key: AssemblyActionKey,
    endpoint: boomerang_federated::EndpointId,
    encoder: Box<dyn runtime::FederatedPayloadEncoder<T>>,
) -> Result<EnclaveConnectionSource<T>, AssemblyError> {
    let mut source_ctx = assembly.add_reactor("con_reactor_src", parent_key, None, (), false);
    if let Some(scope_mode) = scope_mode {
        source_ctx.set_scope_mode(scope_mode)?;
    }
    let input_port = source_ctx.add_input_port::<T>("con_in")?;
    source_ctx
        .add_reaction(None)
        .with_trigger(input_port)
        .with_deferred_reaction_factory(move |runtime_assembly| {
            let (enclave_key, runtime_action_key) = runtime_assembly
                .aliases
                .action_aliases
                .get(target_action_key)
                .expect("Action key");
            let enclave = &runtime_assembly.enclaves[*enclave_key];

            let enclave_key2 = runtime_assembly.aliases.enclave_aliases[target_partition];
            assert_eq!(enclave_key, &enclave_key2, "Temporary cross-check");

            let remote_action_ref = enclave.create_async_action_ref(*runtime_action_key);
            let (outbound, faults) = runtime_assembly
                .federation
                .as_ref()
                .expect("federated sender exists only in a lowered federation")
                .runtime
                .connections()
                .outbound_endpoint(&endpoint)
                .expect("federated endpoint sink was validated before deferred lowering");
            runtime::InterPartitionSenderReactionFn::<T>::new(
                remote_action_ref,
                Box::new(runtime::SerializedInterPartitionEventSink::new(
                    encoder, outbound, faults,
                )),
                None,
            )
            .into()
        })
        .finish()?;
    let reactor_key = source_ctx.finish()?;
    Ok(EnclaveConnectionSource {
        reactor_key,
        input_port,
    })
}

struct EnclaveConnectionTarget<T: runtime::ReactorData, Q: ActionTag> {
    reactor_key: AssemblyReactorKey,
    output_port: TypedPortKey<T, Output>,
    action_key: TypedActionKey<T, Q>,
}

/// Build the target portion
///
/// The receiver-side of is built immediately into the `Assembly`, and consists of an Action that triggers a Reaction
/// that writes to the target port.
fn build_enclave_connection_target<T: runtime::ReactorData + Clone, Q: ActionTag>(
    assembly: &mut Assembly,
    parent_key: Option<AssemblyReactorKey>,
    scope_mode: Option<AssemblyModeKey>,
    after: Option<runtime::Duration>,
) -> Result<EnclaveConnectionTarget<T, Q>, AssemblyError> {
    let mut target_ctx = assembly.add_reactor("con_reactor_tgt", parent_key, None, (), false);
    if let Some(scope_mode) = scope_mode {
        target_ctx.set_scope_mode(scope_mode)?;
    }
    let action_key = target_ctx.add_action::<T, Q>("con_act", after)?;
    let output_port = target_ctx.add_output_port::<T>("con_out")?;

    target_ctx
        .add_reaction(None)
        .with_trigger(action_key)
        .with_effect(output_port)
        .with_deferred_reaction_factory(move |_runtime_assembly| {
            runtime::ConnectionReceiverReactionFn::<T>::default().into()
        })
        .finish()?;

    let reactor_key = target_ctx.finish()?;
    Ok(EnclaveConnectionTarget {
        reactor_key,
        output_port,
        action_key,
    })
}
