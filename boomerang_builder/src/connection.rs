//! Specialized builder for non-port-binding connections between reactors.
//!
//! Non-port-binding connections are connections with a specified delay or between enclaves.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    time::Duration,
};

use boomerang_runtime::CommonContext;
use slotmap::SecondaryMap;

use crate::{
    runtime, ActionTag, BuilderActionKey, BuilderError, BuilderPortKey, BuilderReactorKey,
    EnclaveParts, EnvBuilder, Input, Logical, Output, ParentReactorBuilder, PartitionMap, Physical,
    PortType, Reaction, ReactionBuilderState, ReactionField, ReactorBuilderState, ReactorField,
    TriggerMode, TypedActionKey, TypedPortKey,
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
        while let Some(new_idx) = self
            .inward
            .get(cur_key)
            .and_then(|inward_key| self.inward.get(*inward_key))
        {
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
    fn after(&self) -> Option<Duration>;
    fn physical(&self) -> bool;
    fn build(
        &self,
        env: &mut EnvBuilder,
        partition_map: &mut PartitionMap,
        port_bindings: &mut PortBindings,
    ) -> Result<(), BuilderError>;
}

pub struct ConnectionBuilder<T: runtime::ReactorData> {
    pub(crate) source_key: BuilderPortKey,
    pub(crate) target_key: BuilderPortKey,
    pub(crate) after: Option<Duration>,
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
    fn after(&self) -> Option<Duration> {
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
    ) -> Result<(), BuilderError> {
        let source_port = &env.port_builders[self.source_key()];
        let target_port = &env.port_builders[self.target_key()];

        let source_reactor_key = source_port.parent_reactor_key().unwrap();
        let target_reactor_key = target_port.parent_reactor_key().unwrap();

        let source_partition = partition_map[source_reactor_key];
        let target_partition = partition_map[target_reactor_key];

        // Does the connection cross an enclave boundary?
        if source_partition == target_partition {
            // Simple case, we can just bind them directly
            port_bindings.bind(self.source_key, self.target_key, env)?;
        } else {
            // Ports connected with a delay and/or physical connections are implemented as a pair of Reactions that trigger and react to an action.

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
        }

        Ok(())
    }
}

/// Build the source portion
///
/// The sender-side is build deferred by returning a closure. The BuilderAction must be turned into a runtime Action before the closure is called.
fn build_enclave_connection_source<T: runtime::ReactorData + Clone>(
    env: &mut EnvBuilder,
    parent_key: Option<BuilderReactorKey>,
    target_partition: BuilderReactorKey,
    target_action_key: BuilderActionKey,
) -> Result<(BuilderReactorKey, TypedPortKey<T, Input>), BuilderError> {
    let mut source_builder = env.add_reactor("con_reactor_src", parent_key, None, (), false);
    let input_port = source_builder.add_input_port::<T>("con_in")?;
    let _ = source_builder
        .add_reaction("con_react_src", move |partitions| {
            let partition = partitions.get(target_partition).expect("Target partition");
            let runtime_action_key = partition
                .aliases
                .action_aliases
                .get(target_action_key)
                .expect("Runtime action key");
            let remote_context = partition.enclave.create_send_context();
            let remote_action_ref = partition
                .enclave
                .create_async_action_ref(*runtime_action_key);
            runtime::EnclaveSenderReactionFn::<T>::new(
                remote_context,
                remote_action_ref,
                Some(Duration::from_millis(500)),
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
    after: Option<Duration>,
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
            runtime::EnclaveReceiverReactionFn::<T>::default().into()
        })
        .with_action(action_key, 0, TriggerMode::TriggersAndUses)?
        .with_port(output_port, 0, TriggerMode::EffectsOnly)?
        .finish()?;
    let reactor_key = target_builder.finish()?;
    Ok((reactor_key, output_port, action_key))
}

#[cfg(feature = "disable")]
mod old {
    pub struct DelayedConnectionBuilder<T: runtime::ReactorData, Q: ActionTag, const ENCLAVE: bool> {
        pub(crate) input: TypedPortKey<T, Input>,
        pub(crate) output: TypedPortKey<T, Output>,
        pub(crate) action: TypedActionKey<T, Q>,
    }

    /// We use the `state` to pass the delay duration for the connection.
    impl<T: runtime::ReactorData + Clone, Q: ActionTag, const ENCLAVE: bool> crate::Reactor
        for DelayedConnectionBuilder<T, Q, ENCLAVE>
    {
        type State = Duration;

        fn build(
            name: &str,
            state: Self::State,
            parent: Option<BuilderReactorKey>,
            bank_info: Option<runtime::BankInfo>,
            is_enclave: bool,
            env: &mut EnvBuilder,
        ) -> Result<Self, BuilderError> {
            let mut __builder = env.add_reactor(name, parent, bank_info, (), is_enclave);
            let input =
                <TypedPortKey<T, Input> as ReactorField>::build("input", (), &mut __builder)?;
            let output =
                <TypedPortKey<T, Output> as ReactorField>::build("output", (), &mut __builder)?;
            let action =
                <TypedActionKey<T, Q> as ReactorField>::build("act", Some(state), &mut __builder)?;
            let mut __reactor = Self {
                input,
                output,
                action,
            };
            let _ = <ConnectionSenderReaction<T, ENCLAVE> as Reaction<Self>>::build(
                stringify!(ReactionYIn),
                &__reactor,
                &mut __builder,
            )?
            .finish()?;
            let _ = <ConnectionReceiverReaction<T> as Reaction<Self>>::build(
                stringify!(ReactionAct),
                &__reactor,
                &mut __builder,
            )?
            .finish()?;
            Ok(__reactor)
        }
    }

    /// A Reaction that connects an Input to an Action for a delayed connection.
    struct ConnectionSenderReaction<'a, T: runtime::ReactorData + Clone, const ENCLAVE: bool> {
        input: runtime::InputRef<'a, T>,
        act: runtime::ActionRef<'a, T>,
    }

    impl<T: runtime::ReactorData + Clone, const ENCLAVE: bool> runtime::FromRefs
        for ConnectionSenderReaction<'_, T, ENCLAVE>
    {
        type Marker<'s> = ConnectionSenderReaction<'s, T, ENCLAVE>;

        fn from_refs<'store>(
            ports: boomerang_runtime::Refs<'store, dyn boomerang_runtime::BasePort>,
            _ports_mut: boomerang_runtime::RefsMut<'store, dyn boomerang_runtime::BasePort>,
            actions: boomerang_runtime::RefsMut<'store, dyn boomerang_runtime::BaseAction>,
        ) -> Self::Marker<'store> {
            let input = ports.partition().expect("Input not found");
            let act = actions.partition_mut().expect("Action not found");
            ConnectionSenderReaction { input, act }
        }
    }

    impl<'a, T: runtime::ReactorData + Clone, Q: ActionTag, const ENCLAVE: bool>
        Reaction<DelayedConnectionBuilder<T, Q, ENCLAVE>>
        for ConnectionSenderReaction<'a, T, ENCLAVE>
    {
        fn build<'builder>(
            name: &str,
            reactor: &DelayedConnectionBuilder<T, Q, ENCLAVE>,
            builder: &'builder mut ReactorBuilderState,
        ) -> Result<ReactionBuilderState<'builder>, BuilderError> {
            let mut __reaction = {
                let wrapper =
                    runtime::ReactionAdapter::<ConnectionSenderReaction<T, ENCLAVE>, ()>::default();
                builder.add_reaction(name, wrapper)
            };
            <runtime::InputRef<'a, u32> as ReactionField>::build(
                &mut __reaction,
                reactor.input.into(),
                0,
                TriggerMode::TriggersAndUses,
            )?;
            <runtime::ActionRef<'a> as ReactionField>::build(
                &mut __reaction,
                reactor.action.into(),
                0,
                TriggerMode::EffectsOnly,
            )?;
            Ok(__reaction)
        }
    }

    impl<T: runtime::ReactorData + Clone, const ENCLAVE: bool> runtime::Trigger<()>
        for ConnectionSenderReaction<'_, T, ENCLAVE>
    {
        fn trigger(mut self, ctx: &mut runtime::Context, _state: &mut ()) {
            if ENCLAVE {
                ctx.schedule_action_async(
                    &mut self.act,
                    self.input.clone().expect("Input value not set"),
                    None,
                );
            } else {
                ctx.schedule_action(
                    &mut self.act,
                    self.input.clone().expect("Input value not set"),
                    None,
                );
            }
        }
    }

    /// A Reaction that connects an Action to an Output for a delayed connection.
    struct ConnectionReceiverReaction<'a, T: runtime::ReactorData> {
        act: runtime::ActionRef<'a, T>,
        output: runtime::OutputRef<'a, T>,
    }

    impl<T: runtime::ReactorData> runtime::FromRefs for ConnectionReceiverReaction<'_, T> {
        type Marker<'s> = ConnectionReceiverReaction<'s, T>;

        fn from_refs<'store>(
            _ports: runtime::Refs<'store, dyn runtime::BasePort>,
            ports_mut: runtime::RefsMut<'store, dyn runtime::BasePort>,
            actions: runtime::RefsMut<'store, dyn runtime::BaseAction>,
        ) -> Self::Marker<'store> {
            let act = actions.partition_mut().expect("Action not found");
            let output = ports_mut.partition_mut().expect("Output not found");
            ConnectionReceiverReaction { act, output }
        }
    }

    impl<'a, T: runtime::ReactorData + Clone, Q: ActionTag, const ENCLAVE: bool>
        Reaction<DelayedConnectionBuilder<T, Q, ENCLAVE>> for ConnectionReceiverReaction<'a, T>
    {
        fn build<'builder>(
            name: &str,
            reactor: &DelayedConnectionBuilder<T, Q, ENCLAVE>,
            builder: &'builder mut ReactorBuilderState,
        ) -> Result<ReactionBuilderState<'builder>, BuilderError> {
            let mut __reaction = {
                let wrapper =
                    runtime::ReactionAdapter::<ConnectionReceiverReaction<T>, ()>::default();
                builder.add_reaction(name, wrapper)
            };
            <runtime::OutputRef<'a, T> as ReactionField>::build(
                &mut __reaction,
                reactor.output.into(),
                0,
                TriggerMode::EffectsOnly,
            )?;
            <runtime::ActionRef<'a> as ReactionField>::build(
                &mut __reaction,
                reactor.action.into(),
                0,
                TriggerMode::TriggersAndUses,
            )?;
            Ok(__reaction)
        }
    }

    impl<T: runtime::ReactorData + Clone> runtime::Trigger<()> for ConnectionReceiverReaction<'_, T> {
        fn trigger(mut self, ctx: &mut runtime::Context, _state: &mut ()) {
            *self.output = ctx.get_action_value(&mut self.act).cloned();
        }
    }
}
