//! Runtime data storage

use std::{marker::PhantomPinned, pin::Pin, ptr::NonNull};

use crate::{
    refs::{Refs, RefsMut},
    Action, ActionKey, BasePort, Context, ContextCommon, Deadline, PortKey, Reaction, ReactionKey,
    Reactor, ReactorKey, Tag, TriggerRes,
};

use super::{Env, ReactionGraph};

/// Set of borrows necessary for a single Reaction triggering.
pub struct ReactionTriggerCtx<'store> {
    pub context: &'store mut Context,
    pub reactor: &'store mut Reactor,
    pub reaction: &'store mut Reaction,
    pub ref_ports: Refs<'store, dyn BasePort>,
    pub mut_ports: RefsMut<'store, dyn BasePort>,
    pub actions: RefsMut<'store, Action>,
}

impl<'a> From<&'a mut ReactionTriggerCtxPtrs> for ReactionTriggerCtx<'a> {
    fn from(ptrs: &mut ReactionTriggerCtxPtrs) -> Self {
        let context = unsafe { ptrs.context.as_mut() };
        let reactor = unsafe { ptrs.reactor.as_mut() };
        let reaction = unsafe { ptrs.reaction.as_mut() };

        let ref_ports = Refs::new(&mut ptrs.ref_ports);
        let mut_ports = RefsMut::new(&mut ptrs.mut_ports);
        let actions = RefsMut::new(&mut ptrs.actions);

        Self {
            context,
            reactor,
            reaction,
            ref_ports,
            mut_ports,
            actions,
        }
    }
}

impl<'a> ReactionTriggerCtx<'a> {
    /// Trigger the reaction with the given context and state.
    pub fn trigger(self, tag: Tag) -> &'a TriggerRes {
        tracing::trace!(
            "    Executing {reactor_name}/{reaction_name}.",
            reaction_name = self.reaction.get_name(),
            reactor_name = self.reactor.get_name()
        );

        if let Some(Deadline { deadline, handler }) = self.reaction.deadline.as_ref() {
            let lag = self.context.get_physical_time() - self.context.get_logical_time();
            if lag > *deadline {
                (handler.write().unwrap())();
            }
        }

        self.context.reset_for_reaction(tag);

        self.reaction.body.trigger(
            self.context,
            self.reactor.state.as_mut(),
            self.ref_ports,
            self.mut_ports,
            self.actions,
        );

        &self.context.trigger_res
    }
}

/// Lifetime-erased version of [`ReactionTriggerCtx`]
///
/// This is used to pre-calculate and cache the necessary pointers for each reaction's trigger data.
#[derive(Debug)]
struct ReactionTriggerCtxPtrs {
    context: NonNull<Context>,
    reactor: NonNull<Reactor>,
    reaction: NonNull<Reaction>,
    ref_ports: Vec<NonNull<dyn BasePort>>,
    mut_ports: Vec<NonNull<dyn BasePort>>,
    actions: Vec<NonNull<Action>>,
}

impl Default for ReactionTriggerCtxPtrs {
    fn default() -> Self {
        Self {
            context: NonNull::dangling(),
            reactor: NonNull::dangling(),
            reaction: NonNull::dangling(),
            ref_ports: Vec::new(),
            mut_ports: Vec::new(),
            actions: Vec::new(),
        }
    }
}

unsafe impl Send for ReactionTriggerCtxPtrs {}

#[derive(Debug)]
struct Inner {
    contexts: tinymap::TinySecondaryMap<ReactionKey, Context>,
    reactors: tinymap::TinyMap<ReactorKey, Reactor>,
    reactions: tinymap::TinyMap<ReactionKey, Reaction>,
    actions: tinymap::TinyMap<ActionKey, Action>,
    ports: tinymap::TinyMap<PortKey, Box<dyn BasePort>>,
}

#[derive(Debug)]
pub struct Store {
    inner: Inner,
    /// Internal caches of `ReactionTriggerCtxPtrs`
    caches: tinymap::TinySecondaryMap<ReactionKey, ReactionTriggerCtxPtrs>,
    _pin: PhantomPinned,
}

impl Store {
    /// Create a new `Store` from the given `Env`, `Contexts`, and `ReactionGraph`.
    pub fn new(
        env: Env,
        contexts: tinymap::TinySecondaryMap<ReactionKey, Context>,
        reaction_graph: &ReactionGraph,
    ) -> Pin<Box<Self>> {
        // Create a default `ReactionTriggerCtxPtrs` for each reaction
        let ptrs = env
            .reactions
            .keys()
            .map(|reaction_key| (reaction_key, Default::default()))
            .collect();

        let res = Self {
            inner: Inner {
                contexts,
                reactors: env.reactors,
                reactions: env.reactions,
                actions: env.actions,
                ports: env.ports,
            },
            caches: ptrs,
            _pin: PhantomPinned,
        };

        let mut boxed = Box::new(res);

        let contexts = unsafe {
            boxed
                .inner
                .contexts
                .iter_many_unchecked_mut(boxed.inner.reactions.keys())
                .map(|c| NonNull::new_unchecked(c))
        };

        let reactor_keys = boxed
            .inner
            .reactions
            .keys()
            .map(|reaction_key| reaction_graph.reaction_reactors[reaction_key]);

        let reactors = unsafe {
            boxed
                .inner
                .reactors
                .iter_many_unchecked_ptrs_mut(reactor_keys)
                .map(|r| NonNull::new_unchecked(r))
        };

        let reactions = unsafe {
            boxed
                .inner
                .reactions
                .iter_many_unchecked_mut(boxed.inner.reactions.keys())
                .map(|r| NonNull::new_unchecked(r))
        };

        let action_keys = reaction_graph
            .reaction_actions
            .values()
            .map(|actions| actions.iter());

        let grouped_actions = unsafe {
            action_keys.map(|keys| {
                boxed
                    .inner
                    .actions
                    .iter_many_unchecked_ptrs_mut(keys)
                    .map(|a| NonNull::new_unchecked(a))
                    .collect::<Vec<_>>()
            })
        };

        let port_ref_keys = reaction_graph
            .reaction_use_ports
            .values()
            .map(|ports| ports.iter());

        let port_mut_keys = reaction_graph
            .reaction_effect_ports
            .values()
            .map(|ports| ports.iter());

        let (grouped_ref_ports, grouped_mut_ports) = unsafe {
            boxed
                .inner
                .ports
                .iter_ptr_chunks_split_unchecked(port_ref_keys, port_mut_keys)
        };

        for ((_, cache), context, reactor, reaction, actions, ref_ports, mut_ports) in itertools::izip!(
            boxed.caches.iter_mut(),
            contexts,
            reactors,
            reactions,
            grouped_actions,
            grouped_ref_ports,
            grouped_mut_ports,
        ) {
            unsafe {
                cache.context = context;
                cache.reactor = reactor;
                cache.reaction = reaction;
                cache.actions = actions;
                cache.ref_ports = ref_ports
                    .map(|p| NonNull::new_unchecked(&mut **p as *mut _))
                    .collect();
                cache.mut_ports = mut_ports
                    .map(|p| NonNull::new_unchecked(&mut **p as *mut _))
                    .collect();
            }
        }

        Box::into_pin(boxed)
    }

    /// Returns an `Iterator` of `ReactionTriggerCtx` for each `Reaction` in the given `reaction_keys`.
    ///
    /// This uses the previously stored `ReactionTriggerCtxPtrs`.
    pub unsafe fn iter_borrow_storage<'a>(
        self: &'a mut Pin<Box<Self>>,
        keys: impl Iterator<Item = ReactionKey> + 'a,
    ) -> impl Iterator<Item = ReactionTriggerCtx<'a>> + 'a {
        let ptrs = &mut self.as_mut().get_unchecked_mut().caches;
        ptrs.iter_many_unchecked_mut(keys)
            .map(ReactionTriggerCtx::from)
    }

    /// Returns an `Iterator` of `PortKey`s that currently have a value set.
    pub fn iter_set_port_keys(self: &Pin<Box<Self>>) -> impl Iterator<Item = PortKey> + '_ {
        self.inner
            .ports
            .iter()
            .filter(|&(_, port)| port.is_set())
            .map(|(key, _)| key)
    }

    pub fn reset_ports(self: &mut Pin<Box<Self>>) {
        let store = unsafe { self.as_mut().get_unchecked_mut() };
        store.inner.ports.values_mut().for_each(|p| p.cleanup());
    }

    /// Turn this `Store` back into the `Env` it was built from.
    pub fn into_env(self: Pin<Box<Self>>) -> Env {
        // SAFETY: We are the only owner of the `Store` and we are consuming it, and immediately dropping all the cached pointers.
        let store = unsafe { Pin::into_inner_unchecked(self) };
        Env {
            reactors: store.inner.reactors,
            reactions: store.inner.reactions,
            actions: store.inner.actions,
            ports: store.inner.ports,
        }
    }
}

#[cfg(test)]
pub mod tests {
    use itertools::Itertools;

    use crate::{keepalive, ActionRef, InputRef, OutputRef, Timestamp};

    use super::*;

    /// Create a dummy `Store` for testing containing `Action`s.
    pub fn create_dummy_store(env: Env, reaction_graph: &ReactionGraph) -> Pin<Box<Store>> {
        let reaction_key = env.reactions.keys().next().unwrap();

        let (event_tx, _) = crossbeam_channel::bounded(0);
        let (_, shutdown_rx) = keepalive::channel();

        let contexts = [(
            reaction_key,
            Context::new(Timestamp::now(), None, event_tx, shutdown_rx),
        )]
        .into_iter()
        .collect();

        Store::new(env, contexts, reaction_graph)
    }

    #[test]
    fn test_iter_borrow_storage() {
        let (env, reaction_graph) = crate::env::tests::create_dummy_env();
        let reaction_keys = reaction_graph.reaction_actions.keys().collect_vec();

        let mut store = create_dummy_store(env, &reaction_graph);

        {
            let mut ctx_iter = unsafe { store.iter_borrow_storage(reaction_keys.iter().cloned()) };
            let ctx = ctx_iter.next().unwrap();

            let [a0, a1]: [ActionRef; 2] = ctx.actions.partition_mut().unwrap();
            assert_eq!(a0.name(), "action0");
            assert_eq!(a1.name(), "action1");

            let [p0]: [InputRef<u32>; 1] = ctx.ref_ports.partition().unwrap();
            assert_eq!(p0.name(), "port0");

            let mut p1: OutputRef<u32> = ctx.mut_ports.partition_mut().unwrap();
            assert_eq!(p1.name(), "port1");
            *p1 = Some(42);
        }
        {
            let mut ctx_iter = unsafe { store.iter_borrow_storage(reaction_keys.iter().cloned()) };
            let _res = ctx_iter.next().unwrap().trigger(Tag::now(Timestamp::now()));
        }
    }
}
