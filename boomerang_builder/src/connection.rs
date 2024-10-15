use crate::runtime;

/// A Reaction that connects an Input to an Action for a delayed connection.
pub struct ConnectionSenderReaction<'a, T: runtime::ReactorData + Clone> {
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

impl<T: runtime::ReactorData + Clone> runtime::Trigger<()> for ConnectionSenderReaction<'_, T> {
    fn trigger(mut self, ctx: &mut runtime::Context, _state: &mut ()) {
        self.act
            .schedule(ctx, self.input.clone().expect("Input value not set"), None);
    }
}

/// A Reaction that connects an Action to an Output for a delayed connection.
pub struct ConnectionReceiverReaction<'a, T: runtime::ReactorData> {
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

impl<T: runtime::ReactorData + Clone> runtime::Trigger<()> for ConnectionReceiverReaction<'_, T> {
    fn trigger(mut self, ctx: &mut runtime::Context, _state: &mut ()) {
        *self.output = self.act.get_value(ctx).cloned();
    }
}
