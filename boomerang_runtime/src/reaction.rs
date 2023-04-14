use std::sync::RwLock;

use boomerang_core::time::Timestamp;
use crossbeam_channel::Sender;

use crate::{
    key_set::KeySet,
    keys::{ActionKey, PortKey, ReactionKey, ReactorKey},
    Action, BasePort, Context, Reactor, ReactorState, ScheduledEvent, Tag,
};

pub type ReactionSet = KeySet<ReactionKey>;

pub type IPort<'a> = &'a Box<dyn BasePort>;
pub type OPort<'a> = &'a mut Box<dyn BasePort>;

pub type InputPorts<'a> = &'a [IPort<'a>];
pub type OutputPorts<'a> = &'a mut [OPort<'a>];

pub trait ReactionFn:
    Fn(
        &mut Context,
        &mut dyn ReactorState,
        &[IPort], // Input ports
        &mut [OPort], // Output ports
        &mut [&mut Action], // Actions
    ) + Sync
    + Send
{
}
impl<F> ReactionFn for F where
    F: Fn(&mut Context, &mut dyn ReactorState, &[IPort], &mut [OPort], &mut [&mut Action])
        + Send
        + Sync
{
}

#[derive(Derivative)]
#[derivative(Debug, PartialEq)]
pub struct Deadline {
    deadline: Timestamp,
    #[derivative(PartialEq = "ignore")]
    #[derivative(Debug = "ignore")]
    #[allow(dead_code)]
    handler: RwLock<Box<dyn Fn() + Sync + Send>>,
}

#[derive(Derivative)]
#[derivative(Debug, PartialEq)]
pub struct Reaction {
    name: String,
    /// The Reactor containing this Reaction
    reactor_key: ReactorKey,
    /// Input Ports that trigger this reaction
    input_ports: Vec<PortKey>,
    /// Output Ports that this reaction may set the value of
    output_ports: Vec<PortKey>,
    /// Actions that trigger or can be scheduled by this reaction
    actions: Vec<ActionKey>,
    /// Reaction closure
    #[derivative(PartialEq = "ignore")]
    #[derivative(Debug = "ignore")]
    body: Box<dyn ReactionFn>,
    // Local deadline relative to the time stamp for invocation of the reaction.
    deadline: Option<Deadline>,
}

impl Reaction {
    pub fn new(
        name: String,
        reactor_key: ReactorKey,
        input_ports: Vec<PortKey>,
        output_ports: Vec<PortKey>,
        actions: Vec<ActionKey>,
        body: Box<dyn ReactionFn>,
        deadline: Option<Deadline>,
    ) -> Self {
        Self {
            name,
            reactor_key,
            input_ports,
            output_ports,
            actions,
            body,
            deadline,
        }
    }

    pub fn get_name(&self) -> &str {
        &self.name
    }

    pub fn get_reactor_key(&self) -> ReactorKey {
        self.reactor_key
    }

    pub fn set_reactor_key(&mut self, reactor_key: ReactorKey) {
        self.reactor_key = reactor_key;
    }

    pub fn iter_input_ports(&self) -> std::slice::Iter<PortKey> {
        self.input_ports.iter()
    }

    pub fn iter_output_ports(&self) -> std::slice::Iter<PortKey> {
        self.output_ports.iter()
    }

    pub fn iter_actions(&self) -> std::slice::Iter<ActionKey> {
        self.actions.iter()
    }

    #[tracing::instrument(
        skip(self, start_time, inputs, outputs, async_tx),
        fields(
            reactor = reactor.name,
            name = %self.name,
            tag = %tag,
        )
    )]
    pub fn trigger<'a>(
        &'a self,
        start_time: Timestamp,
        tag: Tag,
        reactor: &'a mut Reactor,
        inputs: &[IPort<'_>],
        outputs: &mut [OPort<'_>],
        async_tx: Sender<ScheduledEvent>,
    ) -> Context {
        let Reactor {
            state,
            actions: action_keys,
            action_triggers,
            ..
        } = reactor;

        let mut ctx = Context::new(start_time, tag, action_triggers, async_tx);

        if let Some(Deadline { deadline, handler }) = self.deadline.as_ref() {
            let lag = ctx.get_physical_time() - ctx.get_logical_time();
            if lag > (*deadline).into() {
                (handler.write().unwrap())();
            }
        }

        // Pull actions from the reaction/reactor
        let mut actions = action_keys
            .iter_many_unchecked_mut(self.iter_actions().copied())
            .collect::<Vec<_>>();

        (self.body)(
            &mut ctx,
            state.as_mut(),
            inputs,
            outputs,
            actions.as_mut_slice(),
        );

        ctx
    }
}
