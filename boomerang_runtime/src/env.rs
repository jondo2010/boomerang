use std::fmt::Display;

use tinymap::{
    chunks::{Chunks, ChunksMut},
    map::{IterMany, IterManyMut},
};

use crate::{
    keys::{PortKey, ReactionKey, ReactorKey},
    BasePort, Reaction, Reactor,
};

#[cfg(feature = "federated")]
use boomerang_federated as federated;

/// Execution level
pub type Level = usize;

/// A paired `ReactionKey` with it's execution level.
pub type LevelReactionKey = (Level, ReactionKey);

/// Extended data from the Federated Environment
#[cfg(feature = "federated")]
pub struct FederateEnv {
    /// Keys for the generated input control trigger actions
    pub input_control_triggers: Vec<crate::keys::ActionKey>,
    /// Keys for the generated network message actions
    pub network_messages: Vec<crate::keys::ActionKey>,
    /// Key for the generated output control trigger action
    pub output_control_trigger: crate::keys::ActionKey,
    /// Federated neighbor structure
    pub neighbors: federated::NeighborStructure,
}

/// `Env` stores the resolved runtime state of all the reactors.
///
/// The reactor heirarchy has been flattened and build by the builder methods.
#[derive(Debug)]
pub struct Env {
    /// The top-level Reactor
    pub top_reactor: ReactorKey,
    /// The runtime set of Reactors
    pub reactors: tinymap::TinyMap<ReactorKey, Reactor>,
    /// The runtime set of Ports
    pub ports: tinymap::TinyMap<PortKey, Box<dyn BasePort>>,
    /// The runtime set of Reactions
    pub reactions: tinymap::TinyMap<ReactionKey, Reaction>,
}

/// Set of borrows necessary for a single Reaction triggering.
pub(crate) struct ReactionTriggerCtx<'a, II>
where
    II: Iterator<Item = PortKey> + Send,
{
    pub(crate) reactor: &'a mut Reactor,
    pub(crate) reaction: &'a Reaction,
    pub(crate) inputs: IterMany<'a, PortKey, Box<dyn BasePort>, II>,
    pub(crate) outputs: IterManyMut<'a, PortKey, Box<dyn BasePort>, II>,
}

/// Container for set of iterators used to build a `ReactionTriggerCtx`
pub(crate) struct ReactionTriggerCtxIter<'a, IReactor, IReaction, IO1, IO2, II>
where
    IReactor: Iterator<Item = &'a mut Reactor>,
    IReaction: Iterator<Item = &'a Reaction>,
    IO1: Iterator<Item = II> + Send,
    IO2: Iterator<Item = II> + Send,
    II: Iterator<Item = PortKey> + Send,
{
    reactors: IReactor,
    reactions: IReaction,
    grouped_inputs: Chunks<'a, PortKey, Box<dyn BasePort>, IO1, II>,
    grouped_outputs: ChunksMut<'a, PortKey, Box<dyn BasePort>, IO2, II>,
}

impl<'a, IReactor, IReaction, IO1, IO2, II> Iterator
    for ReactionTriggerCtxIter<'a, IReactor, IReaction, IO1, IO2, II>
where
    IReactor: Iterator<Item = &'a mut Reactor>,
    IReaction: Iterator<Item = &'a Reaction>,
    IO1: Iterator<Item = II> + Send,
    IO2: Iterator<Item = II> + Send,
    II: Iterator<Item = PortKey> + Send,
{
    type Item = ReactionTriggerCtx<'a, II>;

    fn next(&mut self) -> Option<Self::Item> {
        let reactor = self.reactors.next();
        let reaction = self.reactions.next();
        let inputs = self.grouped_inputs.next();
        let outputs = self.grouped_outputs.next();

        match (reactor, reaction, inputs, outputs) {
            (Some(reactor), Some(reaction), Some(inputs), Some(outputs)) => {
                Some(ReactionTriggerCtx {
                    reactor,
                    reaction,
                    inputs,
                    outputs,
                })
            }
            (None, None, None, None) => None,
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
    #[cfg_attr(feature = "profiling", profiling::function)]
    pub(crate) fn iter_reaction_ctx<'a, I>(
        &'a mut self,
        reaction_keys: I,
    ) -> impl Iterator<Item = ReactionTriggerCtx<'a, impl Iterator<Item = PortKey> + Send + 'a>> + 'a
    where
        I: Iterator<Item = &'a ReactionKey> + Clone + Send + 'a,
    {
        let reactions = reaction_keys.map(|&k| &self.reactions[k]);

        let reactor_keys = reactions.clone().map(|reaction| reaction.get_reactor_key());

        let input_keys = reactions
            .clone()
            .map(|reaction| reaction.iter_input_ports().copied());

        let output_keys = reactions
            .clone()
            .map(|reaction| reaction.iter_output_ports().copied());

        let reactors = self.reactors.iter_many_unchecked_mut(reactor_keys);

        let (inputs, outputs) = self
            .ports
            .iter_chunks_split_unchecked(input_keys, output_keys);

        ReactionTriggerCtxIter {
            reactors,
            reactions,
            grouped_inputs: inputs,
            grouped_outputs: outputs,
        }
    }
}
