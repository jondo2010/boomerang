//! Runtime data storage

use std::{pin::Pin, ptr::NonNull};

use crate::{
    refs::{Refs, RefsMut},
    ActionKey, BaseAction, BasePort, BaseReactor, CommonContext, Context, Deadline, PortKey,
    Reaction, ReactionKey, ReactorData, ReactorKey, Tag, TriggerRes,
};

use super::{Env, ReactionGraph};

/// Set of borrows necessary for a single Reaction triggering.
pub struct ReactionTriggerCtx<'store> {
    pub context: &'store mut Context,
    pub reactor: &'store mut dyn BaseReactor,
    pub reaction: &'store mut Reaction,
    pub actions: RefsMut<'store, dyn BaseAction>,
    pub ref_ports: Refs<'store, dyn BasePort>,
    pub mut_ports: RefsMut<'store, dyn BasePort>,
}

unsafe impl Send for ReactionTriggerCtx<'_> {}

impl<'a> From<&'a mut ReactionTriggerCtxPtrs> for ReactionTriggerCtx<'a> {
    fn from(ptrs: &mut ReactionTriggerCtxPtrs) -> Self {
        let context = unsafe { ptrs.context.as_mut() };
        let reactor = unsafe { ptrs.reactor.unwrap().as_mut() };
        let reaction = unsafe { ptrs.reaction.as_mut() };

        let actions = RefsMut::new(&mut ptrs.actions);
        let ref_ports = Refs::new(&mut ptrs.ref_ports);
        let mut_ports = RefsMut::new(&mut ptrs.mut_ports);

        Self {
            context,
            reactor,
            reaction,
            actions,
            ref_ports,
            mut_ports,
        }
    }
}

impl<'a> ReactionTriggerCtx<'a> {
    /// Trigger the reaction with the given context and state.
    #[tracing::instrument(level = "trace", skip(self, tag), fields(reactor = self.reactor.name(), reaction = self.reaction.get_name()))]
    pub(crate) fn trigger(self, tag: Tag) -> &'a TriggerRes {
        tracing::trace!("Exec");

        if let Some(Deadline { deadline, handler }) = self.reaction.deadline.as_ref() {
            let lag = self.context.get_physical_time() - self.context.get_logical_time();
            if lag > *deadline {
                (handler.write().unwrap())();
            }
        }

        self.context.reset_for_reaction(tag);

        self.reaction.body.trigger(
            self.context,
            self.reactor,
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
    reactor: Option<NonNull<dyn BaseReactor>>,
    reaction: NonNull<Reaction>,
    actions: Vec<NonNull<dyn BaseAction>>,
    ref_ports: Vec<NonNull<dyn BasePort>>,
    mut_ports: Vec<NonNull<dyn BasePort>>,
}

impl Default for ReactionTriggerCtxPtrs {
    fn default() -> Self {
        Self {
            context: NonNull::dangling(),
            reactor: None,
            reaction: NonNull::dangling(),
            actions: Vec::new(),
            ref_ports: Vec::new(),
            mut_ports: Vec::new(),
        }
    }
}

unsafe impl Send for ReactionTriggerCtxPtrs {}

#[derive(Debug)]
struct Inner {
    contexts: tinymap::TinySecondaryMap<ReactionKey, Context>,
    reactors: tinymap::TinyMap<ReactorKey, Box<dyn BaseReactor>>,
    reactions: tinymap::TinyMap<ReactionKey, Reaction>,
    actions: tinymap::TinyMap<ActionKey, Box<dyn BaseAction>>,
    ports: tinymap::TinyMap<PortKey, Box<dyn BasePort>>,
}

#[derive(Debug)]
#[pin_project::pin_project]
pub struct Store {
    #[pin]
    inner: Inner,
    /// Internal caches of `ReactionTriggerCtxPtrs`
    #[pin]
    caches: tinymap::TinySecondaryMap<ReactionKey, ReactionTriggerCtxPtrs>,
}

impl Store {
    /// Create a new `Store` from the given `Env`, `Contexts`, and `ReactionGraph`.
    pub fn new(
        env: Env,
        contexts: tinymap::TinySecondaryMap<ReactionKey, Context>,
        reaction_graph: &ReactionGraph,
    ) -> Pin<Box<Self>> {
        debug_assert!(contexts.len() == env.reactions.len());

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
        };

        // Pin the Box first, then use projection for safe access
        let mut pinned = Box::pin(res);

        // Use pin-project's projection to safely access the pinned fields
        let this = pinned.as_mut().project();
        let inner = this.inner.get_mut();
        let caches = this.caches.get_mut();

        // SAFETY: We're initializing the caches with self-references. This is safe because:
        // 1. The data is already pinned and won't move
        // 2. We're creating pointers to pinned data
        // 3. The Store will remain pinned for its entire lifetime
        unsafe {
            let contexts = inner
                .contexts
                .iter_many_unchecked_mut(inner.reactions.keys())
                .map(|c| NonNull::new_unchecked(c));

            let reactor_keys = inner
                .reactions
                .keys()
                .map(|reaction_key| reaction_graph.reaction_reactors[reaction_key]);

            let reactors = inner
                .reactors
                .iter_many_unchecked_ptrs_mut(reactor_keys)
                .map(|r| NonNull::new_unchecked(&mut **r as *mut _));

            let reactions = inner
                .reactions
                .iter_many_unchecked_mut(inner.reactions.keys())
                .map(|r| NonNull::new_unchecked(r));

            let action_keys = reaction_graph
                .reaction_actions
                .values()
                .map(|actions| actions.iter());

            let (_, grouped_actions) = inner
                .actions
                .iter_ptr_chunks_split_unchecked(std::iter::empty(), action_keys);

            let port_ref_keys = reaction_graph
                .reaction_use_ports
                .values()
                .map(|ports| ports.iter());

            let port_mut_keys = reaction_graph
                .reaction_effect_ports
                .values()
                .map(|ports| ports.iter());

            let (grouped_ref_ports, grouped_mut_ports) = inner
                .ports
                .iter_ptr_chunks_split_unchecked(port_ref_keys, port_mut_keys);

            for ((_, cache), context, reactor, reaction, actions, ref_ports, mut_ports) in itertools::izip!(
                caches.iter_mut(),
                contexts,
                reactors,
                reactions,
                grouped_actions,
                grouped_ref_ports,
                grouped_mut_ports,
            ) {
                cache.context = context;
                cache.reactor = Some(reactor);
                cache.reaction = reaction;
                cache.actions = actions
                    .map(|a| NonNull::new_unchecked(&mut **a as *mut _))
                    .collect();
                cache.ref_ports = ref_ports
                    .map(|p| NonNull::new_unchecked(&mut **p as *mut _))
                    .collect();
                cache.mut_ports = mut_ports
                    .map(|p| NonNull::new_unchecked(&mut **p as *mut _))
                    .collect();
            }
        }

        pinned
    }

    pub fn push_action_value(
        self: &mut Pin<Box<Self>>,
        action_key: ActionKey,
        tag: Tag,
        value: Box<dyn ReactorData>,
    ) {
        // SAFETY: we are projecting to a field, not moving anything from self
        let actions = &mut self.as_mut().project().inner.actions;
        actions[action_key].push_value(tag, value);
    }

    /// Returns an `Iterator` of `ReactionTriggerCtx` for each `Reaction` in the given
    /// `reaction_keys`.
    ///
    /// This uses the previously stored `ReactionTriggerCtxPtrs` which contain `NonNull` pointers
    /// to data within the `Store`'s pinned `inner` fields.
    ///
    /// # Safety
    ///
    /// This method is unsafe because it extends the lifetime of mutable references to the cache
    /// from the projection's temporary lifetime to `'a` (the lifetime of `&'a mut Pin<Box<Self>>`).
    ///
    /// ## Caller Requirements:
    ///
    /// 1. **No Aliasing**: The caller must ensure that no other references (mutable or immutable)
    ///    to the data pointed to by the returned `ReactionTriggerCtx` instances exist while the
    ///    returned iterator is alive. This includes:
    ///    - Other calls to `iter_borrow_storage` with overlapping `ReactionKey`s
    ///    - Direct access to `inner.contexts`, `inner.reactors`, `inner.reactions`, etc.
    ///
    /// 2. **Sequential Processing**: Reactions returned by this iterator should be processed
    ///    sequentially (or with non-overlapping keys in parallel) to avoid multiple mutable
    ///    borrows of the same underlying data.
    ///
    /// 3. **No Store Modification**: The `Store` must not be modified (e.g., through
    ///    `push_action_value`, `reset_ports`, etc.) while the returned iterator or any
    ///    `ReactionTriggerCtx` instances derived from it are alive.
    ///
    /// ## Why This is Sound:
    ///
    /// - The `Store` is pinned, so the data pointed to by the cached `NonNull` pointers
    ///   will not move or be invalidated.
    /// - The `ReactionTriggerCtxPtrs` contain pointers that were created during `Store::new()`
    ///   and point to pinned fields within `inner`.
    /// - Each `ReactionTriggerCtx` provides exclusive mutable access to a specific reaction's
    ///   resources (context, reactor, reaction, actions, ports), which doesn't overlap with
    ///   other reactions' resources when used correctly.
    /// - The lifetime `'a` ensures that the returned contexts cannot outlive the `Store` itself.
    pub unsafe fn iter_borrow_storage<'a>(
        self: &'a mut Pin<Box<Self>>,
        keys: impl Iterator<Item = ReactionKey> + 'a,
    ) -> impl Iterator<Item = ReactionTriggerCtx<'a>> + 'a {
        // SAFETY: We use pin-project's projection to safely access the caches field.
        // The lifetime 'a properly represents the borrow of the Store. This is safe because:
        // 1. The Store is pinned and won't move
        // 2. The caller must uphold the safety requirements documented above
        // 3. The returned ReactionTriggerCtx instances borrow from data that lives as long as
        //    the Store itself (lifetime 'a)
        let caches = self.as_mut().project().caches.get_mut();
        caches
            .iter_many_unchecked_mut(keys)
            .map(ReactionTriggerCtx::from)
    }

    /// Returns an `Iterator` of `PortKey`s that currently have a value set.
    pub fn iter_set_port_keys(self: &Pin<Box<Self>>) -> impl Iterator<Item = PortKey> + '_ {
        self.as_ref()
            .get_ref()
            .inner
            .ports
            .iter()
            .filter(|&(_, port)| port.is_set())
            .map(|(key, _)| key)
    }

    pub fn reset_ports(self: &mut Pin<Box<Self>>) {
        self.as_mut()
            .project()
            .inner
            .ports
            .values_mut()
            .for_each(|p| p.cleanup());
    }

    /// Turn this `Store` back into the `Env` it was built from.
    pub fn into_env(self: Pin<Box<Self>>) -> Env {
        // SAFETY: We are the only owner of the `Store` and we are consuming it, and immediately
        // dropping all the cached pointers.
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

    use crate::{action::ActionCommon, keepalive, ActionRef, EnclaveKey, InputRef, OutputRef};

    use super::*;

    /// Create a dummy `Store` for testing containing `Action`s.
    pub fn create_dummy_store(env: Env, reaction_graph: &ReactionGraph) -> Pin<Box<Store>> {
        let reaction_key = env.reactions.keys().next().unwrap();

        let (event_tx, _) = kanal::bounded(0);
        let (_, shutdown_rx) = keepalive::channel();

        let contexts = [(
            reaction_key,
            Context::new(
                EnclaveKey::default(),
                std::time::Instant::now(),
                None,
                event_tx,
                shutdown_rx,
            ),
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

            let (a0, a1): (ActionRef, ActionRef) = ctx.actions.partition_mut().unwrap();
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
            let _res = ctx_iter.next().unwrap().trigger(Tag::ZERO);
        }
    }
}
