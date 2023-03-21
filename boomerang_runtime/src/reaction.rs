use std::{fmt::Debug, sync::RwLock};

use crate::{
    key_set::KeySet, BasePort, Context, Duration, InternalAction, PortKey, Reactor, ReactorKey,
};

tinymap::key_type!(pub ReactionKey);

pub type ReactionSet = KeySet<ReactionKey>;

pub type IPort<'a> = &'a Box<dyn BasePort>;
pub type OPort<'a> = &'a mut Box<dyn BasePort>;

pub type InputPorts<'a> = &'a [IPort<'a>];
pub type OutputPorts<'a> = &'a mut [OPort<'a>];

pub trait ReactionFn:
    Fn(
        &mut Context,
        &mut Reactor,
        &[IPort], // Input ports
        &mut [OPort], // Output ports
        &[&InternalAction], // Actions
        &mut [&mut InternalAction], // Schedulable Actions
    ) + Sync
    + Send
{
}
impl<F> ReactionFn for F where
    F: Fn(
            &mut Context,
            &mut Reactor,
            &[IPort],
            &mut [OPort],
            &[&InternalAction],
            &mut [&mut InternalAction],
        ) + Send
        + Sync
{
}

#[derive(Derivative)]
#[derivative(Debug, PartialEq)]
pub struct Deadline {
    deadline: Duration,
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
        body: Box<dyn ReactionFn>,
        deadline: Option<Deadline>,
    ) -> Self {
        Self {
            name,
            reactor_key,
            input_ports,
            output_ports,
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

    pub fn trigger(
        &self,
        ctx: &mut Context,
        reactor: &mut Reactor,
        inputs: &[IPort<'_>],
        outputs: &mut [OPort<'_>],
        actions: &[&InternalAction],
        schedulable_actions: &mut [&mut InternalAction],
    ) {
        if let Some(Deadline { deadline, handler }) = self.deadline.as_ref() {
            let lag = ctx.get_physical_time() - ctx.get_logical_time();
            if lag > *deadline {
                (handler.write().unwrap())();
            }
        }

        (self.body)(ctx, reactor, inputs, outputs, actions, schedulable_actions);
    }
}
