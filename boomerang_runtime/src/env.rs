use tinymap::chunks::{Chunks, ChunksMut};

use crate::{
    fmt_utils as fmt, Action, ActionKey, BasePort, PortKey, Reaction, ReactionKey, Reactor,
    ReactorKey,
};

/// Execution level
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
pub struct Level(pub(crate) usize);

impl std::fmt::Display for Level {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "L{}", self.0)
    }
}

impl std::ops::Add<usize> for Level {
    type Output = Self;

    fn add(self, rhs: usize) -> Self::Output {
        Self(self.0 + rhs)
    }
}

impl std::ops::Sub<usize> for Level {
    type Output = Self;

    fn sub(self, rhs: usize) -> Self::Output {
        Self(self.0 - rhs)
    }
}

/// A paired `ReactionKey` with it's execution level.
pub type LevelReactionKey = (Level, ReactionKey);

/// `Env` stores the resolved runtime state of all the reactors.
///
/// The reactor heirarchy has been flattened and build by the builder methods.
pub struct Env {
    /// The runtime set of Reactors
    pub reactors: tinymap::TinyMap<ReactorKey, Reactor>,
    /// The runtime set of Actions
    pub actions: tinymap::TinyMap<ActionKey, Action>,
    /// The runtime set of Ports
    pub ports: tinymap::TinyMap<PortKey, Box<dyn BasePort>>,
    /// The runtime set of Reactions
    pub reactions: tinymap::TinyMap<ReactionKey, Reaction>,
}

/// Maps of triggers for actions and ports. This data is statically resolved by the builder from the reaction graph.
pub struct TriggerMap {
    /// Global action triggers
    pub action_triggers: tinymap::TinySecondaryMap<ActionKey, Vec<LevelReactionKey>>,
    /// Global port triggers
    pub port_triggers: tinymap::TinySecondaryMap<PortKey, Vec<LevelReactionKey>>,
    /// Global startup reactions
    pub startup_reactions: Vec<LevelReactionKey>,
    /// Global shutdown reactions
    pub shutdown_reactions: Vec<LevelReactionKey>,
}

impl std::fmt::Debug for Env {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let reactors = fmt::from_fn(|f| {
            f.debug_map()
                .entries(
                    self.reactors
                        .iter()
                        .map(|(k, reactor)| (format!("{k:?}"), &reactor.name)),
                )
                .finish()
        });

        let actions = fmt::from_fn(|f| {
            let e = self
                .actions
                .iter()
                .map(|(action_key, action)| (format!("{action_key:?}"), action.to_string()));
            f.debug_map().entries(e).finish()
        });

        let ports = fmt::from_fn(|f| {
            f.debug_map()
                .entries(
                    self.ports
                        .iter()
                        .map(|(k, v)| (format!("{k:?}"), v.to_string())),
                )
                .finish()
        });

        let reactions = fmt::from_fn(|f| {
            f.debug_map()
                .entries(self.reactions.iter().map(|(reaction_key, reaction)| {
                    let reaction_dbg = fmt::from_fn(|f| {
                        let use_ports = fmt::from_fn(|f| {
                            f.debug_list()
                                .entries(
                                    reaction
                                        .iter_use_ports()
                                        .map(|port_key| self.ports[*port_key].to_string()),
                                )
                                .finish()
                        });
                        let effect_ports = fmt::from_fn(|f| {
                            f.debug_list()
                                .entries(
                                    reaction
                                        .iter_effect_ports()
                                        .map(|port_key| self.ports[*port_key].to_string()),
                                )
                                .finish()
                        });
                        let actions = fmt::from_fn(|f| {
                            f.debug_list()
                                .entries(
                                    reaction
                                        .iter_actions()
                                        .map(|action_key| self.actions[*action_key].to_string()),
                                )
                                .finish()
                        });

                        f.debug_struct(reaction.get_name())
                            .field("reactor", &self.reactors[reaction.get_reactor_key()].name)
                            .field("use_ports", &use_ports)
                            .field("effect_ports", &effect_ports)
                            .field("actions", &actions)
                            .finish()
                    });
                    (format!("{reaction_key:?}"), reaction_dbg)
                }))
                .finish()
        });

        f.debug_struct("Env")
            .field("reactors", &reactors)
            .field("actions", &actions)
            .field("ports", &ports)
            .field("reactions", &reactions)
            .finish()
    }
}

impl std::fmt::Debug for TriggerMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let action_triggers = fmt::from_fn(|f| {
            let e = self.action_triggers.iter().map(|(action_key, v)| {
                let v = fmt::from_fn(|f| {
                    let e = v.iter().map(|(level, reaction_key)| {
                        (format!("{level:?}"), format!("{reaction_key:?}"))
                    });
                    f.debug_map().entries(e).finish()
                });

                (format!("{action_key:?}"), v)
            });
            f.debug_map().entries(e).finish()
        });

        let port_triggers = fmt::from_fn(|f| {
            let e = self.port_triggers.iter().map(|(port_key, v)| {
                let v = fmt::from_fn(|f| {
                    let e = v.iter().map(|(level, reaction_key)| {
                        (format!("{level:?}"), format!("{reaction_key:?}"))
                    });
                    f.debug_map().entries(e).finish()
                });

                (format!("{port_key:?}"), v)
            });
            f.debug_map().entries(e).finish()
        });

        f.debug_struct("TriggerMap")
            .field("action_triggers", &action_triggers)
            .field("port_triggers", &port_triggers)
            .finish()
    }
}

/// Set of borrows necessary for a single Reaction triggering.
pub(crate) struct ReactionTriggerCtx<'a> {
    pub(crate) reactor: &'a mut Reactor,
    pub(crate) reaction: &'a Reaction,
    pub(crate) actions: &'a mut [&'a mut Action],
    pub(crate) inputs: &'a [&'a Box<dyn BasePort>],
    pub(crate) outputs: &'a mut [&'a mut Box<dyn BasePort>],
}

/// Container for set of iterators used to build a `ReactionTriggerCtx`
pub(crate) struct ReactionTriggerCtxIter<'a, 'bump, IReactor, IReaction, IO1, IO2, IO3, IA, IP>
where
    IReactor: Iterator<Item = &'a mut Reactor>,
    IReaction: Iterator<Item = &'a Reaction>,
    IO1: Iterator<Item = IA> + Send,
    IO2: Iterator<Item = IP> + Send,
    IO3: Iterator<Item = IP> + Send,
    IA: Iterator<Item = ActionKey> + Send,
    IP: Iterator<Item = PortKey> + Send,
{
    bump: &'bump bumpalo::Bump,
    reactors: IReactor,
    reactions: IReaction,
    grouped_actions: ChunksMut<'a, ActionKey, Action, IO1, IA>,
    grouped_inputs: Chunks<'a, PortKey, Box<dyn BasePort>, IO2, IP>,
    grouped_outputs: ChunksMut<'a, PortKey, Box<dyn BasePort>, IO3, IP>,
}

impl<'a, 'bump: 'a, IReactor, IReaction, IO1, IO2, IO3, IA, IP> Iterator
    for ReactionTriggerCtxIter<'a, 'bump, IReactor, IReaction, IO1, IO2, IO3, IA, IP>
where
    IReactor: Iterator<Item = &'a mut Reactor>,
    IReaction: Iterator<Item = &'a Reaction>,
    IO1: Iterator<Item = IA> + Send,
    IO2: Iterator<Item = IP> + Send,
    IO3: Iterator<Item = IP> + Send,
    IA: Iterator<Item = ActionKey> + ExactSizeIterator + Send,
    IP: Iterator<Item = PortKey> + ExactSizeIterator + Send,
{
    type Item = ReactionTriggerCtx<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let reactor = self.reactors.next();
        let reaction = self.reactions.next();
        let actions = self.grouped_actions.next();
        let inputs = self.grouped_inputs.next();
        let outputs = self.grouped_outputs.next();

        match (reactor, reaction, actions, inputs, outputs) {
            (Some(reactor), Some(reaction), Some(actions), Some(inputs), Some(outputs)) => {
                Some(ReactionTriggerCtx {
                    reactor,
                    reaction,
                    actions: self.bump.alloc_slice_fill_iter(actions),
                    inputs: self.bump.alloc_slice_fill_iter(inputs),
                    outputs: self.bump.alloc_slice_fill_iter(outputs),
                })
            }
            (None, None, None, None, None) => None,
            _ => {
                unreachable!("Mismatched iterators in ReactionTriggerCtxIter");
            }
        }
    }
}

#[cfg(feature = "parallel2")]
impl<'a, IReactor, IReaction, IInputs, IOutputs> rayon::iter::ParallelIterator
    for ReactionTriggerCtxIter<'a, IReactor, IReaction, IInputs, IOutputs>
where
    IReactor: Iterator<Item = &'a mut Reactor> + Send,
    IReaction: Iterator<Item = &'a Reaction> + Send,
    IInputs: Iterator<Item = &'a [&'a Box<dyn BasePort>]> + Send,
    IOutputs: Iterator<Item = &'a mut [&'a mut Box<dyn BasePort>]> + Send,
{
    type Item = ReactionTriggerCtx<'a>;

    fn drive_unindexed<C>(self, _consumer: C) -> C::Result
    where
        C: rayon::iter::plumbing::UnindexedConsumer<Self::Item>,
    {
        todo!()
    }
}

impl Env {
    /// Returns an `Iterator` of `ReactionTriggerCtx` for each `Reaction` in the given `reaction_keys`.
    ///
    /// # Safety
    /// The Reactions corresponding to `reaction_keys` must be be independent of each other.
    pub(crate) unsafe fn iter_reaction_ctx<'a, 'bump: 'a, I>(
        &'a mut self,
        bump: &'bump bumpalo::Bump,
        reaction_keys: I,
    ) -> impl Iterator<Item = ReactionTriggerCtx<'a>> + 'a
    where
        I: Iterator<Item = &'a ReactionKey> + ExactSizeIterator + Clone + Send + 'a,
    {
        let reactions = reaction_keys.map(|&k| &self.reactions[k]);

        let reactor_keys = reactions.clone().map(|reaction| reaction.get_reactor_key());

        let action_keys = reactions
            .clone()
            .map(|reaction| reaction.iter_actions().copied());

        let port_keys = reactions
            .clone()
            .map(|reaction| reaction.iter_use_ports().copied());

        let mut_port_keys = reactions
            .clone()
            .map(|reaction| reaction.iter_effect_ports().copied());

        let reactors = self.reactors.iter_many_unchecked_mut(reactor_keys);

        // SAFETY: `action_keys` are guaranteed to guaranteed to be disjoint chunks
        let (_, actions) = unsafe {
            self.actions
                .iter_chunks_split_unchecked(std::iter::empty(), action_keys)
        };

        let (inputs, outputs) = unsafe {
            self.ports
                .iter_chunks_split_unchecked(port_keys, mut_port_keys)
        };

        ReactionTriggerCtxIter {
            bump,
            reactors,
            reactions,
            grouped_actions: actions,
            grouped_inputs: inputs,
            grouped_outputs: outputs,
        }
    }

    pub fn get_reactor_by_name(&self, name: &str) -> Option<&Reactor> {
        self.reactors
            .iter()
            .find(|(_, reactor)| reactor.name == name)
            .map(|(_, reactor)| reactor)
    }
}
