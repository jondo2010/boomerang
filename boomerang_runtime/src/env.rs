use std::fmt::Display;

use slotmap::{SecondaryMap, SlotMap};

use crate::{BasePort, PortKey, Reaction, ReactionKey, Reactor, ReactorKey};

/// Execution level
pub type Level = usize;

/// A paired `ReactionKey` with it's execution level.
pub type LevelReactionKey = (Level, ReactionKey);

/// `Env` stores the resolved runtime state of all the reactors.
///
/// The reactor heirarchy has been flattened and build by the builder methods.
pub struct Env {
    /// The runtime set of Reactors
    pub reactors: tinymap::TinyMap<ReactorKey, Reactor>,
    /// The runtime set of Ports
    pub ports: tinymap::TinyMap<PortKey, Box<dyn BasePort>>,
    /// The runtime set of Reactions
    pub reactions: tinymap::TinyMap<ReactionKey, Reaction>,
}

/// Set of borrows necessary for a single Reaction triggering.
pub(crate) struct ReactionTriggerCtx<'a> {
    pub(crate) reactor: &'a mut Reactor,
    pub(crate) reaction: &'a Reaction,
    pub(crate) inputs: &'a [&'a Box<dyn BasePort>],
    pub(crate) outputs: &'a mut [&'a mut Box<dyn BasePort>],
}

/// Container for set of iterators used to build a `ReactionTriggerCtx`
pub(crate) struct ReactionTriggerCtxIter<'a, IReactor, IReaction, IInputs, IOutputs>
where
    IReactor: Iterator<Item = &'a mut Reactor>,
    IReaction: Iterator<Item = &'a Reaction>,
    IInputs: Iterator<Item = &'a [&'a Box<dyn BasePort>]>,
    IOutputs: Iterator<Item = &'a mut [&'a mut Box<dyn BasePort>]>,
{
    reactors: IReactor,
    reactions: IReaction,
    grouped_inputs: IInputs,
    grouped_outputs: IOutputs,
}

impl<'a, IReactor, IReaction, IInputs, IOutputs> Iterator
    for ReactionTriggerCtxIter<'a, IReactor, IReaction, IInputs, IOutputs>
where
    IReactor: Iterator<Item = &'a mut Reactor>,
    IReaction: Iterator<Item = &'a Reaction>,
    IInputs: Iterator<Item = &'a [&'a Box<dyn BasePort>]>,
    IOutputs: Iterator<Item = &'a mut [&'a mut Box<dyn BasePort>]>,
{
    type Item = ReactionTriggerCtx<'a>;

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
    pub(crate) fn iter_reaction_ctx<'a, I>(
        &'a mut self,
        dep_info: &'a DepInfo,
        reaction_keys: I,
    ) -> ReactionTriggerCtxIter<
        'a,
        impl Iterator<Item = &'a mut Reactor> + 'a,
        impl Iterator<Item = &'a Reaction> + 'a,
        impl Iterator<Item = &'a [&'a Box<dyn BasePort>]> + 'a,
        impl Iterator<Item = &'a mut [&'a mut Box<dyn BasePort>]> + 'a,
    >
    where
        I: Iterator<Item = &'a ReactionKey> + Clone + Send + 'a,
    {
        let reactions = reaction_keys.clone().map(|&k| &self.reactions[k]);

        let reactor_keys = reactions
            .clone()
            .map(|reaction| reaction.get_reactor_key())
            .inspect(|&k| {
                tracing::trace!("Borrowing {k:?}");
            });
        let reactors = self.reactors.iter_many_unchecked_mut(reactor_keys);

        let input_keys = reaction_keys
            .clone()
            .map(|&k| dep_info.reaction_inputs[k].iter());

        let output_keys = reaction_keys
            .clone()
            .map(|&k| dep_info.reaction_outputs[k].iter());

        let (inputs, outputs) = self
            .ports
            .iter_chunks_split_unchecked(input_keys, output_keys);

        // let trig_action_keys = reaction_keys .clone() .map(|&k|
        // dep_info.reaction_trig_actions[k].iter()); let sched_action_keys =
        // reaction_keys.map(|&k| dep_info.reaction_sched_actions[k].iter());

        // let (actions, sched_actions) = disjoint::disjoint_unchecked_chunked(
        //    &mut self.actions,
        //    trig_action_keys,
        //    sched_action_keys,
        //);

        ReactionTriggerCtxIter {
            // Conversion to Vec neccessary, see https://github.com/rust-lang/rust/issues/59878
            reactors,
            reactions,
            grouped_inputs: inputs,
            grouped_outputs: outputs,
        }
    }

    // pub fn find_action_by_name(&self, name: &str) -> Option<(ActionKey, &InternalAction)> {
    //    self.actions
    //        .iter()
    //        .find(|(_, action)| action.get_name().eq(name))
    //}

    /// Return the Reactions in a given Reactor
    pub fn reactions_for_reactor(
        &self,
        reactor_key: ReactorKey,
    ) -> impl Iterator<Item = ReactionKey> + '_ {
        self.reactions
            .iter()
            .filter_map(move |(reaction_key, reaction)| {
                if reaction.get_reactor_key() == reactor_key {
                    Some(reaction_key)
                } else {
                    None
                }
            })
    }
}

/// DepInfo stores immutable dependency information for triggers and reactions, calculated by the
/// builder
#[derive(Debug)]
pub struct DepInfo {
    /// For each Port, a set of Reactions triggered by it.
    pub port_triggers: tinymap::TinySecondaryMap<PortKey, Vec<ReactionKey>>,
    /// For each Reaction, the corresponding level.
    pub reaction_levels: tinymap::TinySecondaryMap<ReactionKey, Level>,
    /// For each Reaction, an ordered list of Ports provided as inputs.
    pub reaction_inputs: tinymap::TinySecondaryMap<ReactionKey, Vec<PortKey>>,
    /// For each Reaction, an ordered list of Ports provided as outputs.
    pub reaction_outputs: tinymap::TinySecondaryMap<ReactionKey, Vec<PortKey>>,
    // For each Reaction, an ordered list of associated Actions that trigger it, unless they also
    // can be scheduled. pub reaction_trig_actions: SecondaryMap<ReactionKey, Vec<ActionKey>>,
    // For each Reaction, an ordered list of associated Actions that can be scheduled.
    // pub reaction_sched_actions: SecondaryMap<ReactionKey, Vec<ActionKey>>,
}

impl DepInfo {
    /// Return the maximum reaction level
    pub fn max_level(&self) -> Level {
        self.reaction_levels.values().max().copied().unwrap_or(0)
    }

    /// Return an iterator of (level, ReactionKey) tuples that are triggered by the given Action
    // pub fn triggered_by_action(
    //    &self,
    //    action_key: ActionKey,
    //) -> impl Iterator<Item = (Level, ReactionKey)> + '_ {
    //    self.action_triggers[action_key]
    //        .iter()
    //        .map(move |&reaction_key| (self.reaction_levels[reaction_key], reaction_key))
    //}

    /// Return an iterator of (level, ReactionKey) tuples that are triggered by the given Port
    pub fn triggered_by_port(
        &self,
        port_key: PortKey,
    ) -> impl Iterator<Item = LevelReactionKey> + '_ {
        self.port_triggers[port_key]
            .iter()
            .map(move |&reaction_key| (self.reaction_levels[reaction_key], reaction_key))
    }
}
