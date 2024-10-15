use crate::{
    Action, ActionData, ActionRef, ActionRefValue, BasePort, FromRefs, InputRef, OutputRef,
    PortData, Refs, RefsMut, Trigger,
};

/// A Reaction that connects an Input to an Action for a delayed connection.
pub struct ConnectionSenderReaction<'a, T: PortData + ActionData> {
    act: ActionRef<'a, T>,
    input: InputRef<'a, T>,
}

impl<T: PortData + ActionData> FromRefs for ConnectionSenderReaction<'_, T> {
    type Marker<'s> = ConnectionSenderReaction<'s, T>;

    fn from_refs<'store>(
        ports: Refs<'store, dyn BasePort>,
        _ports_mut: RefsMut<'store, dyn BasePort>,
        actions: RefsMut<'store, Action>,
    ) -> Self::Marker<'store> {
        let act = actions.partition_mut().expect("Action not found");
        let input = ports.partition().expect("Input not found");
        ConnectionSenderReaction { act, input }
    }
}

impl<T: PortData + ActionData + Clone> Trigger<()> for ConnectionSenderReaction<'_, T> {
    fn trigger(mut self, ctx: &mut crate::Context, _state: &mut ()) {
        self.act.set_value(self.input.clone(), ctx.get_tag());
    }
}

/// A Reaction that connects an Action to an Output for a delayed connection.
pub struct ConnectionReceiverReaction<'a, T: PortData + ActionData> {
    act: ActionRef<'a, T>,
    output: OutputRef<'a, T>,
}

impl<T: PortData + ActionData> FromRefs for ConnectionReceiverReaction<'_, T> {
    type Marker<'s> = ConnectionReceiverReaction<'s, T>;

    fn from_refs<'store>(
        _ports: Refs<'store, dyn BasePort>,
        ports_mut: RefsMut<'store, dyn BasePort>,
        actions: RefsMut<'store, Action>,
    ) -> Self::Marker<'store> {
        let act = actions.partition_mut().expect("Action not found");
        let output = ports_mut.partition_mut().expect("Output not found");
        ConnectionReceiverReaction { act, output }
    }
}

impl<T: PortData + ActionData + Clone> Trigger<()> for ConnectionReceiverReaction<'_, T> {
    fn trigger(mut self, ctx: &mut crate::Context, _state: &mut ()) {
        self.act.get_value_with(ctx.get_tag(), |value| {
            *self.output = value.cloned();
        });
    }
}
