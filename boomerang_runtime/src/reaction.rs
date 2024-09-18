use std::{fmt::Debug, sync::RwLock};

use crossbeam_channel::Sender;

use crate::{
    event::PhysicalEvent, keepalive, key_set::KeySet, Action, BasePort, Context, Duration, Reactor,
    ReactorState, Tag, TriggerRes,
};

tinymap::key_type!(pub ReactionKey);

pub type ReactionSet = KeySet<ReactionKey>;

/// PortRef is the type-erased ref argument passed to the ReactionFn
pub type PortRef<'a> = &'a (dyn BasePort + 'static);
/// PortRefMut is the mutable type-erased ref argument passed to the ReactionFn
pub type PortRefMut<'a> = &'a mut (dyn BasePort + 'static);

pub type ReactionFn = Box<
    dyn for<'a> FnMut(
            &mut Context,
            &'a mut dyn ReactorState,
            &'a [PortRef<'a>],
            &'a mut [PortRefMut<'a>],
            &'a mut [&'a mut Action],
        ) + Sync
        + Send,
>;

pub type HandlerFn = Box<dyn Fn() + Send + Sync>;

pub struct Deadline {
    deadline: Duration,
    #[allow(dead_code)]
    handler: RwLock<HandlerFn>,
}

impl Debug for Deadline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Deadline")
            .field("deadline", &self.deadline)
            .field("handler", &"HandlerFn()")
            .finish()
    }
}

pub struct Reaction {
    name: String,
    /// Reaction closure
    body: ReactionFn,
    // Local deadline relative to the time stamp for invocation of the reaction.
    deadline: Option<Deadline>,
}

impl Debug for Reaction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reaction")
            .field("name", &self.name)
            .field("body", &"ReactionFn()")
            .field("deadline", &self.deadline)
            .finish()
    }
}

impl Reaction {
    pub fn new(name: String, body: ReactionFn, deadline: Option<Deadline>) -> Self {
        Self {
            name,
            body,
            deadline,
        }
    }

    pub fn get_name(&self) -> &str {
        &self.name
    }

    #[allow(clippy::too_many_arguments)]
    #[tracing::instrument(
        skip(self, start_time, ref_ports, mut_ports, actions, async_tx, shutdown_rx),
        fields(
            reactor = reactor.name,
            name = %self.name,
            tag = %tag,
        )
    )]
    pub fn trigger<'a>(
        &'a mut self,
        start_time: crate::Instant,
        tag: Tag,
        reactor: &'a mut Reactor,
        actions: &'a mut [&'a mut Action],
        ref_ports: &'a [PortRef<'a>],
        mut_ports: &'a mut [PortRefMut<'a>],
        async_tx: Sender<PhysicalEvent>,
        shutdown_rx: keepalive::Receiver,
    ) -> TriggerRes {
        let Reactor { state, .. } = reactor;

        let mut ctx = Context::new(start_time, tag, async_tx, shutdown_rx);

        if let Some(Deadline { deadline, handler }) = self.deadline.as_ref() {
            let lag = ctx.get_physical_time() - ctx.get_logical_time();
            if lag > *deadline {
                (handler.write().unwrap())();
            }
        }

        (self.body)(&mut ctx, state.as_mut(), ref_ports, mut_ports, actions);

        ctx.trigger_res
    }
}
