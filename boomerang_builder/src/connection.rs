use std::time::Duration;

use crate::{
    runtime, ActionTag, BuilderError, BuilderReactorKey, EnvBuilder, Input, Logical, Output, Reaction, ReactionBuilderState, ReactionField, ReactorBuilderState, ReactorField, TriggerMode, TypedActionKey, TypedPortKey
};

pub struct ConnectionBuilder<T: runtime::ReactorData, Q: ActionTag> {
    pub(crate) input: TypedPortKey<T, Input>,
    pub(crate) output: TypedPortKey<T, Output>,
    pub(crate) action: TypedActionKey<T, Q>,
}

/// We use the `state` to pass the delay duration for the connection.
impl<T: runtime::ReactorData + Clone, Q: ActionTag> crate::Reactor for ConnectionBuilder<T, Q> {
    type State = Duration;

    fn build(
        name: &str,
        state: Self::State,
        parent: Option<BuilderReactorKey>,
        bank_info: Option<runtime::BankInfo>,
        env: &mut EnvBuilder,
    ) -> Result<Self, BuilderError> {
        let mut __builder = env.add_reactor(name, parent, bank_info, ());
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
        let _ = <ConnectionSenderReaction<T> as Reaction<Self>>::build(
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
struct ConnectionSenderReaction<'a, T: runtime::ReactorData + Clone> {
    input: runtime::InputRef<'a, T>,
    act: runtime::ActionRef<'a, T>,
}

impl<T: runtime::ReactorData + Clone> runtime::FromRefs for ConnectionSenderReaction<'_, T> {
    type Marker<'s> = ConnectionSenderReaction<'s, T>;

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

impl<'a, T: runtime::ReactorData + Clone, Q: ActionTag> Reaction<ConnectionBuilder<T, Q>>
    for ConnectionSenderReaction<'a, T>
{
    fn build<'builder>(
        name: &str,
        reactor: &ConnectionBuilder<T, Q>,
        builder: &'builder mut ReactorBuilderState,
    ) -> Result<ReactionBuilderState<'builder>, BuilderError> {
        let mut __reaction = {
            let wrapper = runtime::ReactionAdapter::<ConnectionSenderReaction<T>, ()>::default();
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

impl<T: runtime::ReactorData + Clone> runtime::Trigger<()> for ConnectionSenderReaction<'_, T> {
    fn trigger(mut self, ctx: &mut runtime::Context, _state: &mut ()) {
        self.act
            .schedule(ctx, self.input.clone().expect("Input value not set"), None);
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

impl<'a, T: runtime::ReactorData + Clone, Q: ActionTag> Reaction<ConnectionBuilder<T, Q>>
    for ConnectionReceiverReaction<'a, T>
{
    fn build<'builder>(
        name: &str,
        reactor: &ConnectionBuilder<T, Q>,
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
        *self.output = self.act.get_value(ctx).cloned();
    }
}
