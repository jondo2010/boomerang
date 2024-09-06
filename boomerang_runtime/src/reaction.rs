use std::{fmt::Debug, sync::RwLock};

use crossbeam_channel::Sender;

use crate::{
    event::PhysicalEvent, keepalive, key_set::KeySet, Action, ActionKey, BasePort, Context,
    Duration, PortKey, Reactor, ReactorKey, ReactorState, Tag, TriggerRes,
};

tinymap::key_type!(pub ReactionKey);

pub type ReactionSet = KeySet<ReactionKey>;

pub type PortRef<'a> = &'a dyn BasePort;
pub type PortRefMut<'a> = &'a mut dyn BasePort;

pub type ReactionFn = Box<
    dyn Fn(&mut Context, &mut dyn ReactorState, &[PortRef], &mut [PortRefMut], &mut [&mut Action])
        + Sync
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
    /// The Reactor containing this Reaction
    reactor_key: ReactorKey,
    /// Ports that this reaction may read from (uses + triggers)
    use_ports: Vec<PortKey>,
    /// Output Ports that this reaction may set the value of
    effect_ports: Vec<PortKey>,
    /// Actions that trigger or can be scheduled by this reaction
    actions: Vec<ActionKey>,
    /// Reaction closure
    body: ReactionFn,
    // Local deadline relative to the time stamp for invocation of the reaction.
    deadline: Option<Deadline>,
}

impl Debug for Reaction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reaction")
            .field("name", &self.name)
            .field("reactor_key", &self.reactor_key)
            .field("use_ports", &self.use_ports)
            .field("effect_ports", &self.effect_ports)
            .field("actions", &self.actions)
            .field("body", &"ReactionFn()")
            .field("deadline", &self.deadline)
            .finish()
    }
}

impl Reaction {
    pub fn new(
        name: String,
        reactor_key: ReactorKey,
        use_ports: Vec<PortKey>,
        effect_ports: Vec<PortKey>,
        actions: Vec<ActionKey>,
        body: ReactionFn,
        deadline: Option<Deadline>,
    ) -> Self {
        Self {
            name,
            reactor_key,
            use_ports,
            effect_ports,
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

    /// Get an iterator over the ports that this reaction may read from.
    pub fn iter_use_ports(&self) -> std::slice::Iter<PortKey> {
        self.use_ports.iter()
    }

    /// Get an iterator over the ports that this reaction may write to.
    pub fn iter_effect_ports(&self) -> std::slice::Iter<PortKey> {
        self.effect_ports.iter()
    }

    /// Get an iterator over the actions that this reaction may trigger or receive.
    pub fn iter_actions(&self) -> std::slice::Iter<ActionKey> {
        self.actions.iter()
    }

    #[allow(clippy::too_many_arguments)]
    #[tracing::instrument(
        skip(self, start_time, inputs, outputs, actions, async_tx, shutdown_rx),
        fields(
            reactor = reactor.name,
            name = %self.name,
            tag = %tag,
        )
    )]
    pub fn trigger<'a>(
        &'a self,
        start_time: crate::Instant,
        tag: Tag,
        reactor: &'a mut Reactor,
        actions: &mut [&mut Action],
        inputs: &[PortRef<'_>],
        outputs: &mut [PortRefMut<'_>],
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

        (self.body)(&mut ctx, state.as_mut(), inputs, outputs, actions);

        ctx.trigger_res
    }
}
