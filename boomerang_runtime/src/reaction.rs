use std::{fmt::Debug, sync::RwLock, time::Duration};

use crate::{key_set::KeySet, Action, BasePort, Context, ContextCommon, Reactor, ReactorState};

tinymap::key_type!(pub ReactionKey);

pub type ReactionSet = KeySet<ReactionKey>;

/// PortRef is the type-erased ref argument passed to the ReactionFn
pub type PortRef<'a> = &'a (dyn BasePort + 'static);
/// PortRefMut is the mutable type-erased ref argument passed to the ReactionFn
pub type PortRefMut<'a> = &'a mut (dyn BasePort + 'static);

pub type PortSlice<'a> = &'a [PortRef<'a>];

pub type PortSliceMut<'a> = &'a mut [PortRefMut<'a>];

pub type ActionSliceMut<'a> = &'a mut [&'a mut Action];

pub type ReactionFn = Box<
    dyn for<'a> FnMut(
            &mut Context,
            &'a mut dyn ReactorState,
            PortSlice<'a>,
            PortSliceMut<'a>,
            ActionSliceMut<'a>,
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
    /// Local deadline relative to the time stamp for invocation of the reaction.
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
        skip(self, ref_ports, mut_ports, actions),
        fields(
            reactor = reactor.get_name(),
            name = %self.name,
            tag = %ctx.tag,
        )
    )]
    pub fn trigger<'a>(
        &mut self,
        ctx: &mut Context,
        reactor: &'a mut Reactor,
        actions: ActionSliceMut<'a>,
        ref_ports: PortSlice<'a>,
        mut_ports: PortSliceMut<'a>,
    ) {
        let Reactor { state, .. } = reactor;

        if let Some(Deadline { deadline, handler }) = self.deadline.as_ref() {
            let lag = ctx.get_physical_time() - ctx.get_logical_time();
            if lag > *deadline {
                (handler.write().unwrap())();
            }
        }

        (self.body)(ctx, state.as_mut(), ref_ports, mut_ports, actions);
    }
}
