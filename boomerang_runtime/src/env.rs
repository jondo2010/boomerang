use std::fmt::Display;

use itertools::Itertools;
use rayon::iter::ParallelBridge;
use slotmap::{SecondaryMap, SlotMap};

use crate::{
    disjoint, sched, ActionKey, BasePort, InternalAction, PortKey, Reaction, ReactionKey,
    ReactorKey, ReactorState,
};

pub type Level = usize;

pub struct Env {
    /// The runtime set of Reactors
    pub reactors: SlotMap<ReactorKey, Box<dyn ReactorState>>,
    /// The runtime set of Ports
    pub ports: SlotMap<PortKey, Box<dyn BasePort>>,
    /// The runtime set of Actions
    pub actions: SlotMap<ActionKey, InternalAction>,
    /// The runtime set of Reactions
    pub reactions: SlotMap<ReactionKey, Reaction>,
}

/// Set of borrows necessary for a single Reaction triggering.
pub(crate) struct ReactionTriggerCtx<'a> {
    pub(crate) reactor: &'a mut dyn ReactorState,
    pub(crate) reaction: &'a Reaction,
    pub(crate) inputs: &'a [&'a dyn BasePort],
    pub(crate) outputs: &'a mut [&'a mut dyn BasePort],
    pub(crate) actions: &'a [&'a InternalAction],
    pub(crate) schedulable_actions: &'a mut [&'a mut InternalAction],
}

/// Container for set of iterators used to build a `ReactionTriggerCtx`
pub(crate) struct ReactionTriggerCtxIter<
    'a,
    IReactor,
    IReaction,
    IInputs,
    IOutputs,
    IActions,
    ISchedActions,
> where
    IReactor: Iterator<Item = &'a mut dyn ReactorState>,
    IReaction: Iterator<Item = &'a Reaction>,
    IInputs: Iterator<Item = &'a [&'a dyn BasePort]>,
    IOutputs: Iterator<Item = &'a mut [&'a mut dyn BasePort]>,
    IActions: Iterator<Item = &'a [&'a InternalAction]>,
    ISchedActions: Iterator<Item = &'a mut [&'a mut InternalAction]>,
{
    reactors: IReactor,
    reactions: IReaction,
    grouped_inputs: IInputs,
    grouped_outputs: IOutputs,
    actions: IActions,
    schedulable_actions: ISchedActions,
}

impl<'a, IReactor, IReaction, IInputs, IOutputs, IActions, ISchedActions> Iterator
    for ReactionTriggerCtxIter<'a, IReactor, IReaction, IInputs, IOutputs, IActions, ISchedActions>
where
    IReactor: Iterator<Item = &'a mut dyn ReactorState>,
    IReaction: Iterator<Item = &'a Reaction>,
    IInputs: Iterator<Item = &'a [&'a dyn BasePort]>,
    IOutputs: Iterator<Item = &'a mut [&'a mut dyn BasePort]>,
    IActions: Iterator<Item = &'a [&'a InternalAction]>,
    ISchedActions: Iterator<Item = &'a mut [&'a mut InternalAction]>,
{
    type Item = ReactionTriggerCtx<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let reactor = self.reactors.next();
        let reaction = self.reactions.next();
        let inputs = self.grouped_inputs.next();
        let outputs = self.grouped_outputs.next();
        let actions = self.actions.next();
        let schedulable_actions = self.schedulable_actions.next();

        match (
            reactor,
            reaction,
            inputs,
            outputs,
            actions,
            schedulable_actions,
        ) {
            (
                Some(reactor),
                Some(reaction),
                Some(inputs),
                Some(outputs),
                Some(actions),
                Some(schedulable_actions),
            ) => Some(ReactionTriggerCtx {
                reactor,
                reaction,
                inputs,
                outputs,
                actions,
                schedulable_actions,
            }),
            _ => None,
        }
    }
}

impl<'a, IReactor, IReaction, IInputs, IOutputs, IActions, ISchedActions>
    rayon::iter::ParallelIterator
    for ReactionTriggerCtxIter<'a, IReactor, IReaction, IInputs, IOutputs, IActions, ISchedActions>
where
    IReactor: Iterator<Item = &'a mut dyn ReactorState> + Send,
    IReaction: Iterator<Item = &'a Reaction> + Send,
    IInputs: Iterator<Item = &'a [&'a dyn BasePort]> + Send,
    IOutputs: Iterator<Item = &'a mut [&'a mut dyn BasePort]> + Send,
    IActions: Iterator<Item = &'a [&'a InternalAction]> + Send,
    ISchedActions: Iterator<Item = &'a mut [&'a mut InternalAction]> + Send,
{
    type Item = ReactionTriggerCtx<'a>;

    fn drive_unindexed<C>(self, consumer: C) -> C::Result
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
        impl Iterator<Item = &'a mut dyn ReactorState> + 'a,
        impl Iterator<Item = &'a Reaction> + 'a,
        impl Iterator<Item = &'a [&'a dyn BasePort]> + 'a,
        impl Iterator<Item = &'a mut [&'a mut dyn BasePort]> + 'a,
        impl Iterator<Item = &'a [&'a InternalAction]> + 'a,
        impl Iterator<Item = &'a mut [&'a mut InternalAction]> + 'a,
    >
    where
        I: Iterator<Item = &'a ReactionKey> + Clone + Send + 'a,
    {
        let reactions = reaction_keys.clone().map(|&k| &self.reactions[k]);

        let reactor_keys = reactions.clone().map(|reaction| reaction.get_reactor_key());
        let reactor_keys = reactor_keys.collect_vec();
        let (_, reactors): (_, Box<[&mut dyn ReactorState]>) = unsafe {
            disjoint::disjoint_unchecked(
                &mut self.reactors,
                std::iter::empty(),
                reactor_keys.iter().cloned(),
            )
        };

        let input_keys = reaction_keys
            .clone()
            .map(|&k| dep_info.reaction_inputs[k].iter());

        let output_keys = reaction_keys
            .clone()
            .map(|&k| dep_info.reaction_outputs[k].iter());

        let (inputs, outputs) =
            disjoint::disjoint_unchecked_chunked(&mut self.ports, input_keys, output_keys);

        let trig_action_keys = reaction_keys
            .clone()
            .map(|&k| dep_info.reaction_trig_actions[k].iter());

        let sched_action_keys = reaction_keys
            .clone()
            .map(|&k| dep_info.reaction_sched_actions[k].iter());

        let (actions, sched_actions) = disjoint::disjoint_unchecked_chunked(
            &mut self.actions,
            trig_action_keys,
            sched_action_keys,
        );

        ReactionTriggerCtxIter {
            // Conversion to Vec neccessary, see https://github.com/rust-lang/rust/issues/59878
            reactors: Vec::from(reactors).into_iter(),
            reactions,
            grouped_inputs: inputs,
            grouped_outputs: outputs,
            actions: actions,
            schedulable_actions: sched_actions,
        }
    }

    pub fn find_action_by_name(&self, name: &str) -> Option<(ActionKey, &InternalAction)> {
        self.actions
            .iter()
            .find(|(_, action)| action.get_name().eq(name))
    }

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

    pub fn get_reactor<T: ReactorState>(&self, reactor_key: ReactorKey) -> Option<&T> {
        self.reactors
            .get(reactor_key)
            .and_then(|reactor| reactor.downcast_ref())
    }
}

/// DepInfo stores immutable dependency information for triggers and reactions, calculated by the
/// builder
#[derive(Debug)]
pub struct DepInfo {
    /// For each Port, a set of Reactions triggered by it.
    pub port_triggers: SecondaryMap<PortKey, Vec<ReactionKey>>,
    /// For each Action, a set of Reactions triggered by it.
    pub action_triggers: SecondaryMap<ActionKey, Vec<ReactionKey>>,
    /// For each Reaction, the corresponding level.
    pub reaction_levels: SecondaryMap<ReactionKey, Level>,
    /// For each Reaction, an ordered list of Ports provided as inputs.
    pub reaction_inputs: SecondaryMap<ReactionKey, Vec<PortKey>>,
    /// For each Reaction, an ordered list of Ports provided as outputs.
    pub reaction_outputs: SecondaryMap<ReactionKey, Vec<PortKey>>,
    /// For each Reaction, an ordered list of associated Actions that trigger it, unless they also
    /// can be scheduled.
    pub reaction_trig_actions: SecondaryMap<ReactionKey, Vec<ActionKey>>,
    /// For each Reaction, an ordered list of associated Actions that can be scheduled.
    pub reaction_sched_actions: SecondaryMap<ReactionKey, Vec<ActionKey>>,
}

impl DepInfo {
    /// Return the maximum reaction level
    pub fn max_level(&self) -> Level {
        self.reaction_levels.values().max().copied().unwrap_or(0)
    }

    /// Return an iterator of (level, ReactionKey) tuples that are triggered by the given Action
    pub fn triggered_by_action(
        &self,
        action_key: ActionKey,
    ) -> impl Iterator<Item = (Level, ReactionKey)> + '_ {
        self.action_triggers[action_key]
            .iter()
            .map(move |&reaction_key| (self.reaction_levels[reaction_key], reaction_key))
    }

    /// Return an iterator of (level, ReactionKey) tuples that are triggered by the given Port
    pub fn triggered_by_port(
        &self,
        port_key: PortKey,
    ) -> impl Iterator<Item = (Level, ReactionKey)> + '_ {
        self.port_triggers[port_key]
            .iter()
            .map(move |&reaction_key| (self.reaction_levels[reaction_key], reaction_key))
    }
}

/// Utility function to check consistency between Env and DepInfo structs.
pub fn check_consistency(env: &Env, dep_info: &DepInfo) {
    for port_key in env.ports.keys() {
        assert!(
            dep_info.port_triggers.contains_key(port_key),
            "PortKey {:?} missing in dep_info.port_triggers!",
            port_key
        );
    }

    for action_key in env.actions.keys() {
        assert!(
            dep_info.action_triggers.contains_key(action_key),
            "ActionKey {:?} missing in dep_info.action_triggers!",
            action_key
        );
    }

    for reaction_key in env.reactions.keys() {
        assert!(
            dep_info.reaction_levels.contains_key(reaction_key),
            "ReactionKey {:?} missing in dep_info.reaction_levels!",
            reaction_key
        );
        assert!(
            dep_info.reaction_inputs.contains_key(reaction_key),
            "ReactionKey {:?} missing in dep_info.reaction_inputs!",
            reaction_key
        );
        assert!(
            dep_info.reaction_outputs.contains_key(reaction_key),
            "ReactionKey {:?} missing in dep_info.reaction_outputs!",
            reaction_key
        );
        assert!(
            dep_info.reaction_trig_actions.contains_key(reaction_key),
            "ReactionKey {:?} missing in dep_info.reaction_trig_actions!",
            reaction_key
        );
    }
}

/// Print debug info about an Env/DepInfo pair.
pub fn debug_info(env: &Env, dep_info: &DepInfo) {
    // Which Reactions are triggered by each Action
    for (action_key, action) in env.actions.iter() {
        let mut action_pairs: Vec<_> = dep_info.triggered_by_action(action_key).collect();
        if action_pairs.len() > 0 {
            action_pairs.sort_by_key(|(level, _)| *level);
            println!("Action {:?} ({}) triggers:", action_key, action.get_name());
            for (level, reaction_key) in action_pairs {
                println!(
                    "  {level}: {:?} ({})",
                    reaction_key,
                    env.reactions[reaction_key].get_name()
                );
            }
        }
    }

    // Which Reactions are triggered by each port
    for (port_key, port) in env.ports.iter() {
        let mut port_pairs: Vec<_> = dep_info.triggered_by_port(port_key).collect();
        if port_pairs.len() > 0 {
            port_pairs.sort_by_key(|(level, _)| *level);
            println!("{port} triggers:");
            for (level, reaction_key) in port_pairs {
                println!(
                    "  {level}: {:?} ({})",
                    reaction_key,
                    env.reactions[reaction_key].get_name()
                );
            }
        }
    }

    for (reaction_key, reaction) in env.reactions.iter() {
        println!("{reaction:?}");
        if !dep_info.reaction_inputs[reaction_key].is_empty() {
            println!("  inputs:");
            for &port_key in dep_info.reaction_inputs[reaction_key].iter() {
                println!("   . {}", env.ports[port_key]);
            }
        }
        if !dep_info.reaction_outputs[reaction_key].is_empty() {
            println!("  outputs:");
            for &port_key in dep_info.reaction_outputs[reaction_key].iter() {
                println!("   . {}", env.ports[port_key]);
            }
        }
        if !dep_info.reaction_trig_actions[reaction_key].is_empty() {
            println!("  triggers:");
            for &action_key in dep_info.reaction_trig_actions[reaction_key].iter() {
                println!("   . {}", env.actions[action_key]);
            }
        }
        if !dep_info.reaction_sched_actions[reaction_key].is_empty() {
            println!("  schedulable actions:");
            for &action_key in dep_info.reaction_sched_actions[reaction_key].iter() {
                println!("   . {}", env.actions[action_key]);
            }
        }
    }
}
