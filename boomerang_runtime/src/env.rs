use std::fmt::Display;

use tinymap::{
    chunks::{Chunks, ChunksMut},
    map::{IterMany, IterManyMut},
};

use crate::{Action, ActionKey, BasePort, PortKey, Reaction, ReactionKey, Reactor, ReactorKey};

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
#[derive(Debug)]
pub struct Env {
    /// The runtime set of Reactors
    pub reactors: tinymap::TinyMap<ReactorKey, Reactor>,
    /// The runtime set of Actions
    pub actions: tinymap::TinyMap<ActionKey, Action>,
    /// The runtime set of Ports
    pub ports: tinymap::TinyMap<PortKey, Box<dyn BasePort>>,
    /// The runtime set of Reactions
    pub reactions: tinymap::TinyMap<ReactionKey, Reaction>,

    /// Global action triggers
    pub action_triggers: tinymap::TinySecondaryMap<ActionKey, Vec<LevelReactionKey>>,
}

/// Set of borrows necessary for a single Reaction triggering.
pub(crate) struct ReactionTriggerCtx<'a, IA, IP>
where
    IA: Iterator<Item = ActionKey> + Send,
    IP: Iterator<Item = PortKey> + Send,
{
    pub(crate) reactor: &'a mut Reactor,
    pub(crate) reaction: &'a Reaction,
    pub(crate) actions: IterManyMut<'a, ActionKey, Action, IA>,
    pub(crate) inputs: IterMany<'a, PortKey, Box<dyn BasePort>, IP>,
    pub(crate) outputs: IterManyMut<'a, PortKey, Box<dyn BasePort>, IP>,
}

/// Container for set of iterators used to build a `ReactionTriggerCtx`
pub(crate) struct ReactionTriggerCtxIter<'a, IReactor, IReaction, IO1, IO2, IO3, IA, IP>
where
    IReactor: Iterator<Item = &'a mut Reactor>,
    IReaction: Iterator<Item = &'a Reaction>,
    IO1: Iterator<Item = IA> + Send,
    IO2: Iterator<Item = IP> + Send,
    IO3: Iterator<Item = IP> + Send,
    IA: Iterator<Item = ActionKey> + Send,
    IP: Iterator<Item = PortKey> + Send,
{
    reactors: IReactor,
    reactions: IReaction,
    grouped_actions: ChunksMut<'a, ActionKey, Action, IO1, IA>,
    grouped_inputs: Chunks<'a, PortKey, Box<dyn BasePort>, IO2, IP>,
    grouped_outputs: ChunksMut<'a, PortKey, Box<dyn BasePort>, IO3, IP>,
}

impl<'a, IReactor, IReaction, IO1, IO2, IO3, IA, IP> Iterator
    for ReactionTriggerCtxIter<'a, IReactor, IReaction, IO1, IO2, IO3, IA, IP>
where
    IReactor: Iterator<Item = &'a mut Reactor>,
    IReaction: Iterator<Item = &'a Reaction>,
    IO1: Iterator<Item = IA> + Send,
    IO2: Iterator<Item = IP> + Send,
    IO3: Iterator<Item = IP> + Send,
    IA: Iterator<Item = ActionKey> + Send,
    IP: Iterator<Item = PortKey> + Send,
{
    type Item = ReactionTriggerCtx<'a, IA, IP>;

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
                    actions,
                    inputs,
                    outputs,
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

impl Display for Env {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Environment {\n")?;
        f.write_str("}\n")?;
        Ok(())
    }
}

impl Env {
    /// Return an `Iterator` of reactions sensitive to `Startup` actions.
    pub fn iter_startup_events(&self) -> impl Iterator<Item = &[LevelReactionKey]> {
        self.actions.iter().filter_map(|(action_key, action)| {
            if let Action::Startup = action {
                Some(self.action_triggers[action_key].as_slice())
            } else {
                None
            }
        })
    }

    /// Return an `Iterator` of reactions sensitive to `Shutdown` actions.
    pub fn iter_shutdown_events(&self) -> impl Iterator<Item = &[LevelReactionKey]> {
        self.actions.iter().filter_map(|(action_key, action)| {
            if let Action::Shutdown { .. } = action {
                Some(self.action_triggers[action_key].as_slice())
            } else {
                None
            }
        })
    }

    pub(crate) fn iter_reaction_ctx<'a, I>(
        &'a mut self,
        reaction_keys: I,
    ) -> impl Iterator<
        Item = ReactionTriggerCtx<
            'a,
            impl Iterator<Item = ActionKey> + Send + 'a,
            impl Iterator<Item = PortKey> + Send + 'a,
        >,
    > + 'a
    where
        I: Iterator<Item = &'a ReactionKey> + Clone + Send + 'a,
    {
        let reactions = reaction_keys.map(|&k| &self.reactions[k]);

        let reactor_keys = reactions.clone().map(|reaction| reaction.get_reactor_key());

        let action_keys = reactions
            .clone()
            .map(|reaction| reaction.iter_actions().copied());

        let input_keys = reactions
            .clone()
            .map(|reaction| reaction.iter_input_ports().copied());

        let output_keys = reactions
            .clone()
            .map(|reaction| reaction.iter_output_ports().copied());

        let reactors = self.reactors.iter_many_unchecked_mut(reactor_keys);

        let (_, actions) = self
            .actions
            .iter_chunks_split_unchecked(std::iter::empty(), action_keys);

        let (inputs, outputs) = self
            .ports
            .iter_chunks_split_unchecked(input_keys, output_keys);

        ReactionTriggerCtxIter {
            reactors,
            reactions,
            grouped_actions: actions,
            grouped_inputs: inputs,
            grouped_outputs: outputs,
        }
    }
}
