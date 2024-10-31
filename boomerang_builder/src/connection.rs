//! Specialized builder for non-port-binding connections between reactors.
//!
//! Non-port-binding connections are connections with a specified delay or between enclaves.

use std::{collections::BTreeSet, time::Duration};

use boomerang_runtime::CommonContext;
use slotmap::SecondaryMap;

use crate::{
    runtime, ActionTag, BuilderError, BuilderPortKey, BuilderReactorKey, EnvBuilder, Input,
    Logical, Output, ParentReactorBuilder, Physical, PortType, Reaction, ReactionBuilderState,
    ReactionField, ReactorBuilderState, ReactorField, TriggerMode, TypedActionKey, TypedPortKey,
};

#[derive(Debug, Default)]
pub struct PortBindings {
    inward: SecondaryMap<BuilderPortKey, BuilderPortKey>,
    outward: SecondaryMap<BuilderPortKey, BTreeSet<BuilderPortKey>>,
}

impl PortBindings {
    fn bind(
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
                match (source_ancestor, target_ancestor) {
                    (Some(key_a), Some(key_b)) if key_a == key_b => {
                        // Valid
                    }
                    _ => {
                        return Err(BuilderError::PortConnectionError {
                            source_key,
                            target_key,
                            what: "An output port A may only be bound to an input port B if both ports belong to reactors in the same hierarichal level.".into(),
                        });
                    }
                }
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
        port_bindings: &mut PortBindings,
    ) -> Result<(), BuilderError> {
        let source_port = &env.port_builders[self.source_key];
        let target_port = &env.port_builders[self.target_key];

        let source_reactor = &env.reactor_builders[source_port.parent_reactor_key().unwrap()];
        let target_reactor = &env.reactor_builders[target_port.parent_reactor_key().unwrap()];

        // Does the connection cross an enclave boundary?
        let is_enclave = source_port.parent_reactor_key() != target_port.parent_reactor_key()
            && (source_reactor.is_enclave || target_reactor.is_enclave);

        if self.after.is_none() && !self.physical && !is_enclave {
            // Simple case: if the ports are in the same reactor, we can just bind them directly
            port_bindings.bind(self.source_key, self.target_key, env)?;
        } else {
            // Ports connected with a delay and/or physical connections are implemented as a pair of Reactions that trigger and react to an action.
            let (input_port, output_port) = self.build_delayed_connection(env)?;
            port_bindings.bind(self.source_key, input_port.into(), env)?;
            port_bindings.bind(output_port.into(), self.target_key, env)?;
        }

        Ok(())
    }
}

impl<T: runtime::ReactorData + Clone> ConnectionBuilder<T> {
    fn build_delayed_connection(
        &self,
        env: &mut EnvBuilder,
    ) -> Result<(TypedPortKey<T, Input>, TypedPortKey<T, Output>), BuilderError> {
        let source_port = &env.port_builders[self.source_key];
        let target_port = &env.port_builders[self.target_key];

        let parent_reactor_key = env
            .common_reactor_key(source_port, target_port)
            .ok_or(BuilderError::PortConnectionError {
            source_key: self.source_key,
            target_key: self.target_key,
            what:
                "Ports must belong to the same reactor or a common parent reactor to be connected"
                    .to_owned(),
        })?;

        let source_fqn = env.port_fqn(self.source_key, false)?;
        let target_fqn = env.port_fqn(self.target_key, false)?;
        let reactor_name = format!("connection_{source_fqn}->{target_fqn}");

        let (input_port, output_port) = if self.physical {
            let reactor = <DelayedConnectionBuilder<T, Physical, false> as crate::Reactor>::build(
                &reactor_name,
                self.after.unwrap_or_default(),
                Some(parent_reactor_key),
                None,
                false,
                env,
            )?;
            (reactor.input, reactor.output)
        } else {
            let reactor = <DelayedConnectionBuilder<T, Logical, false> as crate::Reactor>::build(
                &reactor_name,
                self.after.unwrap_or_default(),
                Some(parent_reactor_key),
                None,
                false,
                env,
            )?;
            (reactor.input, reactor.output)
        };

        Ok((input_port, output_port))
    }
}

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
        let input = <TypedPortKey<T, Input> as ReactorField>::build("input", (), &mut __builder)?;
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
    Reaction<DelayedConnectionBuilder<T, Q, ENCLAVE>> for ConnectionSenderReaction<'a, T, ENCLAVE>
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
            let wrapper = runtime::ReactionAdapter::<ConnectionReceiverReaction<T>, ()>::default();
            builder.add_reaction(name, wrapper)
        };
        <runtime::InputRef<'a, u32> as ReactionField>::build(
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
