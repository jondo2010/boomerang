//! Inner module for `Env` and `ReactionGraph` implementation details.

use tinymap::map;

use crate::{
    Action, ActionKey, ActionSliceMut, BasePort, Context, PortKey, PortSlice, PortSliceMut,
    Reaction, ReactionKey, Reactor,
};

use super::{Env, ReactionGraph};

/// Set of borrows necessary for a single Reaction triggering.
pub struct ReactionTriggerCtx<'a> {
    pub context: &'a mut Context,
    pub reactor: &'a mut Reactor,
    pub reaction: &'a mut Reaction,
    pub actions: ActionSliceMut<'a>,
    pub ref_ports: PortSlice<'a>,
    pub mut_ports: PortSliceMut<'a>,
}

/// Container for set of iterators used to build a `ReactionTriggerCtx`
pub(crate) struct ReactionTriggerCtxIter<
    'a,
    'bump,
    IContext,
    IReactor,
    IReaction,
    IO1,
    IO2,
    IO3,
    IA,
    IP,
> where
    IContext: Iterator<Item = &'a mut Context>,
    IReactor: Iterator<Item = &'a mut Reactor>,
    IReaction: Iterator<Item = &'a mut Reaction>,
    IO1: Iterator<Item = IA> + Send,
    IO2: Iterator<Item = IP> + Send,
    IO3: Iterator<Item = IP> + Send,
    IA: Iterator<Item = ActionKey> + Send,
    IP: Iterator<Item = PortKey> + Send,
{
    bump: &'bump bumpalo::Bump,
    contexts: IContext,
    reactors: IReactor,
    reactions: IReaction,
    grouped_actions: map::ChunksMut<'a, ActionKey, Action, IO1, IA>,
    grouped_ref_ports: map::Chunks<'a, PortKey, Box<dyn BasePort>, IO2, IP>,
    grouped_mut_ports: map::ChunksMut<'a, PortKey, Box<dyn BasePort>, IO3, IP>,
}

impl<'a, 'bump: 'a, IContext, IReactor, IReaction, IO1, IO2, IO3, IA, IP> Iterator
    for ReactionTriggerCtxIter<'a, 'bump, IContext, IReactor, IReaction, IO1, IO2, IO3, IA, IP>
where
    IContext: Iterator<Item = &'a mut Context>,
    IReactor: Iterator<Item = &'a mut Reactor>,
    IReaction: Iterator<Item = &'a mut Reaction>,
    IO1: Iterator<Item = IA> + Send,
    IO2: Iterator<Item = IP> + Send,
    IO3: Iterator<Item = IP> + Send,
    IA: Iterator<Item = ActionKey> + ExactSizeIterator + Send,
    IP: Iterator<Item = PortKey> + ExactSizeIterator + Send,
{
    type Item = ReactionTriggerCtx<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let context = self.contexts.next();
        let reactor = self.reactors.next();
        let reaction = self.reactions.next();
        let actions = self.grouped_actions.next();
        let ref_ports = self.grouped_ref_ports.next();
        let mut_ports = self.grouped_mut_ports.next();

        match (context, reactor, reaction, actions, ref_ports, mut_ports) {
            (
                Some(context),
                Some(reactor),
                Some(reaction),
                Some(actions),
                Some(ref_ports),
                Some(mut_ports),
            ) => Some(ReactionTriggerCtx {
                context,
                reactor,
                reaction,
                actions: self.bump.alloc_slice_fill_iter(actions),
                ref_ports: self.bump.alloc_slice_fill_iter(ref_ports.map(|p| &**p)),
                mut_ports: self.bump.alloc_slice_fill_iter(mut_ports.map(|p| &mut **p)),
            }),
            (None, None, None, None, None, None) => None,
            _ => {
                unreachable!("Mismatched iterators in ReactionTriggerCtxIter");
            }
        }
    }
}

#[derive(Debug)]
pub struct InnerEnv<'env> {
    pub env: &'env mut Env,
    pub contexts: tinymap::TinySecondaryMap<ReactionKey, Context>,
}

impl<'env> InnerEnv<'env> {
    /// Returns an `Iterator` of `ReactionTriggerCtx` for each `Reaction` in the given `reaction_keys`.
    ///
    /// # Safety
    /// The Reactions in `reaction_keys` must be be independent of each other (disjoint).
    pub unsafe fn iter_reaction_ctx<'a, 'bump: 'a, I>(
        &'a mut self,
        reaction_graph: &'a ReactionGraph,
        bump: &'bump bumpalo::Bump,
        reaction_keys: I,
    ) -> impl Iterator<Item = ReactionTriggerCtx<'a>> + 'a
    where
        I: Iterator<Item = ReactionKey> + ExactSizeIterator + Clone + Send + 'a,
    {
        let port_keys = reaction_keys
            .clone()
            .map(|reaction_key| reaction_graph.reaction_use_ports[reaction_key].iter());

        let mut_port_keys = reaction_keys
            .clone()
            .map(|reaction_key| reaction_graph.reaction_effect_ports[reaction_key].iter());

        let action_keys = reaction_keys
            .clone()
            .map(|reaction_key| reaction_graph.reaction_actions[reaction_key].iter());

        let reactor_keys = reaction_keys
            .clone()
            .map(|reaction_key| reaction_graph.reaction_reactors[reaction_key]);

        let contexts = self.contexts.iter_many_unchecked_mut(reaction_keys.clone());

        // SAFETY: reactor_keys are guaranteed to be disjoint
        let reactors = self.env.reactors.iter_many_unchecked_mut(reactor_keys);

        // SAFETY: reaction_keys are guaranteed to be disjoint
        let reactions = self.env.reactions.iter_many_unchecked_mut(reaction_keys);

        // SAFETY: action_keys are guaranteed to be disjoint chunks
        let (_, grouped_actions) = unsafe {
            self.env
                .actions
                .iter_chunks_split_unchecked(std::iter::empty(), action_keys)
        };

        let (grouped_ref_ports, grouped_mut_ports) = unsafe {
            self.env
                .ports
                .iter_chunks_split_unchecked(port_keys, mut_port_keys)
        };

        ReactionTriggerCtxIter {
            bump,
            contexts,
            reactors,
            reactions,
            grouped_actions,
            grouped_ref_ports,
            grouped_mut_ports,
        }
    }

    pub fn iter_set_ports(&self) -> impl Iterator<Item = (PortKey, &Box<dyn BasePort>)> {
        self.env.ports.iter().filter(|&(_, port)| port.is_set())
    }

    pub fn reset_ports(&mut self) {
        for p in self.env.ports.values_mut() {
            p.cleanup();
        }
    }
}
