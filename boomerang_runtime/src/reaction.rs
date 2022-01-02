use std::{fmt::Debug, sync::RwLock};

use crate::{
    key_set::KeySet, ActionMut, BasePort, Context, Duration, InternalAction, ReactorKey,
    ReactorState,
};

slotmap::new_key_type! {
    pub struct ReactionKey;
}

pub type ReactionSet = KeySet<ReactionKey>;

pub type InputPorts<'a> = &'a [&'a dyn BasePort];
pub type OutputPorts<'a> = &'a mut [&'a mut dyn BasePort];

pub trait ReactionFn:
    Fn(
        &mut Context,
        &mut dyn ReactorState,
        &[&dyn BasePort],           // Input ports
        &mut [&mut dyn BasePort],   // Output ports
        &[&InternalAction], // Actions
        &mut [&mut InternalAction], // Schedulable Actions
    ) + Sync
    + Send
{
}
impl<F> ReactionFn for F where
    F: Fn(
            &mut Context,
            &mut dyn ReactorState,
            &[&dyn BasePort],
            &mut [&mut dyn BasePort],
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
    handler: RwLock<Box<dyn Fn() + Sync + Send>>,
}

#[derive(Derivative)]
#[derivative(Debug, PartialEq)]
pub struct Reaction {
    name: String,
    /// The Reactor containing this Reaction
    reactor_key: ReactorKey,
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
        body: Box<dyn ReactionFn>,
        deadline: Option<Deadline>,
    ) -> Self {
        Self {
            name,
            reactor_key,
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

    pub fn trigger(
        &self,
        ctx: &mut Context,
        reactor: &mut dyn ReactorState,
        inputs: &[&dyn BasePort],
        outputs: &mut [&mut dyn BasePort],
        actions: &[&InternalAction],
        schedulable_actions: &mut [&mut InternalAction],
    ) {
        // match self.deadline.as_ref() {
        // Some(Deadline { deadline, handler }) => {
        // let lag = ctx.get_physical_time() - ctx.get_logical_time();
        // if lag > *deadline {
        // (handler.write().unwrap())(ctx);
        // }
        // }
        // _ => {}
        // }

        (self.body)(ctx, reactor, inputs, outputs, actions, schedulable_actions);
    }
}
