use std::{collections::BinaryHeap, pin::Pin};

use tinymap::Key as _;

use super::queue::EventQueue;
use crate::{
    event::ScheduledActionValue, store::Store, Duration, Level, ModeTransitionRequest,
    ReactionGraph, ReactionKey, ReactionSet, ReactionSetLimits, ReactorKey, ScopeKey, Tag,
    TransitionKind,
};

#[derive(Debug)]
struct ScopeClockState {
    activation_global: Tag,
    activation_local: Tag,
    allow_activation_tag: bool,
    suspended_local: Tag,
    frontier_epoch: u64,
}

impl ScopeClockState {
    fn new(active: bool) -> Self {
        Self {
            activation_global: Tag::ZERO,
            activation_local: Tag::ZERO,
            allow_activation_tag: active,
            suspended_local: Tag::ZERO,
            frontier_epoch: 0,
        }
    }

    fn local_to_global(&self, local_tag: Tag) -> Tag {
        if self.activation_global == self.activation_local && self.allow_activation_tag {
            return local_tag;
        }

        local_to_global(
            self.activation_global,
            self.activation_local,
            self.allow_activation_tag,
            local_tag,
        )
    }

    fn global_to_local(&self, global_tag: Tag) -> Tag {
        if self.activation_global == self.activation_local {
            return global_tag;
        }

        global_to_local(self.activation_global, self.activation_local, global_tag)
    }
}

fn global_to_local(activation_global: Tag, activation_local: Tag, global_tag: Tag) -> Tag {
    if activation_global == activation_local {
        return global_tag;
    }

    let elapsed = global_tag.offset() - activation_global.offset();
    let offset = activation_local.offset() + elapsed;
    let microstep = if global_tag.offset() == activation_global.offset()
        && global_tag.microstep() >= activation_global.microstep()
    {
        activation_local.microstep() + (global_tag.microstep() - activation_global.microstep())
    } else {
        global_tag.microstep()
    };

    Tag::new(offset, microstep)
}

fn local_to_global(
    activation_global: Tag,
    activation_local: Tag,
    allow_activation_tag: bool,
    local_tag: Tag,
) -> Tag {
    if activation_global == activation_local && allow_activation_tag {
        return local_tag;
    }

    let elapsed = local_tag.offset() - activation_local.offset();
    let offset = activation_global.offset() + elapsed;
    let microstep = if local_tag.offset() == activation_local.offset()
        && local_tag.microstep() >= activation_local.microstep()
    {
        activation_global.microstep() + (local_tag.microstep() - activation_local.microstep())
    } else {
        local_tag.microstep()
    };
    let mut global_tag = Tag::new(offset, microstep);

    if global_tag < activation_global || (global_tag == activation_global && !allow_activation_tag)
    {
        global_tag = activation_global.delay(Duration::ZERO);
    }

    global_tag
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ScopeFrontierEntry {
    global_tag: Tag,
    scope: ScopeKey,
    epoch: u64,
}

impl Ord for ScopeFrontierEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.global_tag
            .cmp(&other.global_tag)
            .then(self.scope.index().cmp(&other.scope.index()))
            .reverse()
    }
}

impl PartialOrd for ScopeFrontierEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug)]
pub(super) struct ReadyEvent {
    pub(super) tag: Tag,
    pub(super) reactions: ReactionSet,
    pub(super) terminal: bool,
}

#[derive(Debug)]
pub(super) struct EventManager {
    root: EventQueue,
    scope_active: tinymap::TinySecondaryMap<ScopeKey, bool>,
    scope_ever_active: tinymap::TinySecondaryMap<ScopeKey, bool>,
    scope_startup_fired: tinymap::TinySecondaryMap<ScopeKey, bool>,
    scope_clocks: tinymap::TinySecondaryMap<ScopeKey, ScopeClockState>,
    scope_queues: tinymap::TinySecondaryMap<ScopeKey, EventQueue>,
    frontier: BinaryHeap<ScopeFrontierEntry>,
    free_reaction_sets: Vec<ReactionSet>,
    reaction_set_limits: ReactionSetLimits,
    has_local_scopes: bool,
}

impl EventManager {
    pub(super) fn new(
        reaction_set_limits: ReactionSetLimits,
        reaction_graph: &ReactionGraph,
        store: &Pin<Box<Store>>,
    ) -> Self {
        let root = EventQueue::new(reaction_set_limits.clone());
        let mut scope_active = tinymap::TinySecondaryMap::new();
        let mut scope_ever_active = tinymap::TinySecondaryMap::new();
        let mut scope_startup_fired = tinymap::TinySecondaryMap::new();
        let mut scope_clocks = tinymap::TinySecondaryMap::new();
        let mut scope_queues = tinymap::TinySecondaryMap::new();

        for scope in reaction_graph.scopes.keys() {
            let active = store.scope_is_active(reaction_graph, scope);
            scope_active.insert(scope, active);
            scope_ever_active.insert(scope, active);
            scope_startup_fired.insert(scope, active);
            scope_clocks.insert(scope, ScopeClockState::new(active));
            scope_queues.insert(scope, EventQueue::new(reaction_set_limits.clone()));
        }

        Self {
            root,
            scope_active,
            scope_ever_active,
            scope_startup_fired,
            scope_clocks,
            scope_queues,
            frontier: BinaryHeap::new(),
            free_reaction_sets: Vec::new(),
            reaction_set_limits,
            has_local_scopes: !reaction_graph.modes.is_empty(),
        }
    }

    pub(super) fn push_event<I>(&mut self, tag: Tag, reactions: I, terminal: bool)
    where
        I: IntoIterator<Item = (Level, ReactionKey)>,
    {
        self.root.push_event(tag, reactions, terminal);
    }

    pub(super) fn push_action_event<I>(
        &mut self,
        action_key: crate::ActionKey,
        tag: Tag,
        reactions: I,
        terminal: bool,
        reaction_graph: &ReactionGraph,
    ) where
        I: IntoIterator<Item = (Level, ReactionKey)>,
    {
        let action_value = ScheduledActionValue {
            key: action_key,
            stored_tag: tag,
        };
        if !self.has_local_scopes {
            self.root.push_action_event(tag, None, reactions, terminal);
            return;
        }

        let scope = reaction_graph.action_scopes[action_key];
        if !reaction_graph.action_is_logical[action_key]
            || Self::scope_uses_global_time(reaction_graph, scope)
        {
            self.root.push_action_event(tag, None, reactions, terminal);
            return;
        }

        let local_tag = self.scope_clocks[scope].global_to_local(tag);
        self.scope_queues[scope].push_action_event(
            local_tag,
            Some(action_value),
            reactions,
            terminal,
        );
        self.refresh_frontier(scope);
    }

    fn push_local_action_event<I>(
        &mut self,
        scope: ScopeKey,
        local_tag: Tag,
        action_value: ScheduledActionValue,
        reactions: I,
        terminal: bool,
        reaction_graph: &ReactionGraph,
    ) where
        I: IntoIterator<Item = (Level, ReactionKey)>,
    {
        if Self::scope_uses_global_time(reaction_graph, scope) {
            self.root
                .push_action_event(action_value.stored_tag, None, reactions, terminal);
            return;
        }

        self.scope_queues[scope].push_action_event(
            local_tag,
            Some(action_value),
            reactions,
            terminal,
        );
        self.refresh_frontier(scope);
    }

    pub(super) fn peek_tag(&mut self) -> Option<Tag> {
        if !self.has_local_scopes {
            return self.root.peek_tag();
        }

        let root_tag = self.root.peek_tag();
        let local_tag = self.peek_frontier_tag();
        match (root_tag, local_tag) {
            (Some(root), Some(local)) => Some(root.min(local)),
            (Some(root), None) => Some(root),
            (None, Some(local)) => Some(local),
            (None, None) => None,
        }
    }

    pub(super) fn pop_next_event(&mut self) -> Option<ReadyEvent> {
        if !self.has_local_scopes {
            let event = self.root.pop_next_event()?;
            return Some(ReadyEvent {
                tag: event.tag,
                reactions: event.reactions,
                terminal: event.terminal,
            });
        }

        let tag = self.peek_tag()?;
        let mut ready = ReadyEvent {
            tag,
            reactions: self.next_reaction_set(),
            terminal: false,
        };

        if self.root.peek_tag() == Some(tag) {
            let event = self.root.pop_next_event().unwrap();
            ready.reactions.merge(&event.reactions);
            ready.terminal = ready.terminal || event.terminal;
            self.root.recycle_reaction_set(event.reactions);
        }

        while self.peek_frontier_tag() == Some(tag) {
            let frontier = self.frontier.pop().unwrap();
            let event = self.scope_queues[frontier.scope].pop_next_event().unwrap();

            ready.reactions.merge(&event.reactions);
            ready.terminal = ready.terminal || event.terminal;

            self.scope_queues[frontier.scope].recycle_reaction_set(event.reactions);
            self.refresh_frontier(frontier.scope);
        }

        Some(ready)
    }

    pub(super) fn shutdown(&mut self) {
        self.root.shutdown();
    }

    pub(super) fn return_reaction_set(&mut self, reaction_set: ReactionSet) {
        if self.has_local_scopes {
            let mut reaction_set = reaction_set;
            reaction_set.clear();
            self.free_reaction_sets.push(reaction_set);
        } else {
            self.root.recycle_reaction_set(reaction_set);
        }
    }

    pub(super) fn apply_transition(
        &mut self,
        reactor_key: ReactorKey,
        request: &ModeTransitionRequest,
        store: &mut Pin<Box<Store>>,
        reaction_graph: &ReactionGraph,
        current_tag: Tag,
    ) {
        let target_scope = reaction_graph.mode_scopes[request.target];

        if matches!(request.transition, TransitionKind::Reset) {
            self.reset_scope_subtree(target_scope, store, reaction_graph);
            store.reset_child_modes_in_scope(reaction_graph, target_scope);
        }

        store.set_mode(reactor_key, request.target);
        let startup_scopes = self.sync_active_scopes(
            store,
            reaction_graph,
            current_tag,
            target_scope,
            request.transition,
        );
        self.schedule_startup_reactions(
            &startup_scopes,
            store,
            reaction_graph,
            current_tag.delay(Duration::ZERO),
        );

        if matches!(request.transition, TransitionKind::Reset) {
            self.schedule_reset_timer_startups(target_scope, store, reaction_graph);
            self.schedule_reset_reactions(
                target_scope,
                reaction_graph,
                current_tag.delay(Duration::ZERO),
            );
        }
    }

    fn reset_scope_subtree(
        &mut self,
        root_scope: ScopeKey,
        store: &mut Pin<Box<Store>>,
        reaction_graph: &ReactionGraph,
    ) {
        for &scope in reaction_graph
            .modal_schedule_index
            .scope_descendants(root_scope)
        {
            self.scope_queues[scope].clear();
            let clock = &mut self.scope_clocks[scope];
            clock.suspended_local = Tag::ZERO;
            clock.activation_local = Tag::ZERO;
            clock.frontier_epoch = clock.frontier_epoch.wrapping_add(1);
        }

        for &action_key in reaction_graph
            .modal_schedule_index
            .scope_logical_actions(root_scope)
        {
            store.clear_action_values(action_key);
        }
    }

    fn sync_active_scopes(
        &mut self,
        store: &mut Pin<Box<Store>>,
        reaction_graph: &ReactionGraph,
        current_tag: Tag,
        reset_root: ScopeKey,
        transition: TransitionKind,
    ) -> Vec<ScopeKey> {
        let activation_global = current_tag;
        let mut startup_scopes = Vec::new();

        for scope in reaction_graph.scopes.keys() {
            let new_active = store.scope_is_active(reaction_graph, scope);
            let reset = matches!(transition, TransitionKind::Reset)
                && Self::scope_is_descendant_or_self(reaction_graph, scope, reset_root);

            match (self.scope_active[scope], new_active) {
                (true, false) => {
                    let clock = &mut self.scope_clocks[scope];
                    clock.suspended_local = clock.global_to_local(current_tag);
                    self.scope_active[scope] = false;
                    clock.frontier_epoch = clock.frontier_epoch.wrapping_add(1);
                }
                (false, true) => {
                    self.scope_active[scope] = true;
                    self.scope_ever_active[scope] = true;
                    if !self.scope_startup_fired[scope] {
                        self.scope_startup_fired[scope] = true;
                        startup_scopes.push(scope);
                    }
                    let clock = &mut self.scope_clocks[scope];
                    clock.activation_global = activation_global;
                    clock.allow_activation_tag = false;
                    if reset {
                        clock.activation_local = Tag::ZERO;
                        clock.suspended_local = Tag::ZERO;
                    } else {
                        clock.activation_local = clock.suspended_local;
                    }
                    clock.frontier_epoch = clock.frontier_epoch.wrapping_add(1);
                    let activation_global = clock.activation_global;
                    let activation_local = clock.activation_local;
                    let allow_activation_tag = clock.allow_activation_tag;
                    self.scope_queues[scope].rebase_action_values(store, |local_tag| {
                        local_to_global(
                            activation_global,
                            activation_local,
                            allow_activation_tag,
                            local_tag,
                        )
                    });
                    self.refresh_frontier(scope);
                }
                (true, true) if reset => {
                    self.scope_ever_active[scope] = true;
                    let clock = &mut self.scope_clocks[scope];
                    clock.activation_global = activation_global;
                    clock.activation_local = Tag::ZERO;
                    clock.allow_activation_tag = false;
                    clock.suspended_local = Tag::ZERO;
                    clock.frontier_epoch = clock.frontier_epoch.wrapping_add(1);
                    self.refresh_frontier(scope);
                }
                _ => {}
            }
        }

        startup_scopes
    }

    fn schedule_startup_reactions(
        &mut self,
        scopes: &[ScopeKey],
        store: &mut Pin<Box<Store>>,
        reaction_graph: &ReactionGraph,
        tag: Tag,
    ) {
        if scopes.is_empty() {
            return;
        }

        let has_startup_reactions = scopes.iter().any(|&scope| {
            !reaction_graph
                .modal_schedule_index
                .scope_startup_reactions(scope)
                .is_empty()
        });
        if !has_startup_reactions {
            return;
        }

        for &scope in scopes {
            for reaction in reaction_graph
                .modal_schedule_index
                .scope_startup_reactions(scope)
            {
                store.push_action_value(reaction.action, tag, Box::new(()));
            }
        }

        self.push_event(
            tag,
            scopes.iter().flat_map(|&scope| {
                reaction_graph
                    .modal_schedule_index
                    .scope_startup_reactions(scope)
                    .iter()
                    .map(|reaction| reaction.reaction)
            }),
            false,
        );
    }

    fn schedule_reset_timer_startups(
        &mut self,
        root_scope: ScopeKey,
        store: &mut Pin<Box<Store>>,
        reaction_graph: &ReactionGraph,
    ) {
        for &(action_key, local_tag) in reaction_graph
            .modal_schedule_index
            .scope_timer_startups(root_scope)
        {
            let scope = reaction_graph.action_scopes[action_key];
            let global_tag = if Self::scope_uses_global_time(reaction_graph, scope) {
                local_tag
            } else {
                self.scope_clocks[scope].local_to_global(local_tag)
            };
            store.push_action_value(action_key, global_tag, Box::new(()));
            let downstream = reaction_graph.action_triggers[action_key].iter().copied();
            self.push_local_action_event(
                scope,
                local_tag,
                ScheduledActionValue {
                    key: action_key,
                    stored_tag: global_tag,
                },
                downstream,
                false,
                reaction_graph,
            );
        }
    }

    fn schedule_reset_reactions(
        &mut self,
        root_scope: ScopeKey,
        reaction_graph: &ReactionGraph,
        tag: Tag,
    ) {
        let reset_reactions = reaction_graph
            .modal_schedule_index
            .scope_reset_reactions(root_scope);
        if !reset_reactions.is_empty() {
            self.push_event(tag, reset_reactions.iter().copied(), false);
        }
    }

    fn next_reaction_set(&mut self) -> ReactionSet {
        self.free_reaction_sets
            .pop()
            .unwrap_or_else(|| ReactionSet::new(&self.reaction_set_limits))
    }

    fn refresh_frontier(&mut self, scope: ScopeKey) {
        let clock = &mut self.scope_clocks[scope];
        clock.frontier_epoch = clock.frontier_epoch.wrapping_add(1);
        if !self.scope_active[scope] {
            return;
        }

        let Some(local_tag) = self.scope_queues[scope].peek_tag() else {
            return;
        };
        let global_tag = self.scope_clocks[scope].local_to_global(local_tag);
        self.frontier.push(ScopeFrontierEntry {
            global_tag,
            scope,
            epoch: self.scope_clocks[scope].frontier_epoch,
        });
    }

    fn peek_frontier_tag(&mut self) -> Option<Tag> {
        loop {
            let entry = *self.frontier.peek()?;
            let clock = &self.scope_clocks[entry.scope];
            if !self.scope_active[entry.scope] || clock.frontier_epoch != entry.epoch {
                self.frontier.pop();
                continue;
            }

            let Some(local_tag) = self.scope_queues[entry.scope].peek_tag() else {
                self.frontier.pop();
                continue;
            };
            if clock.local_to_global(local_tag) != entry.global_tag {
                self.frontier.pop();
                continue;
            }

            return Some(entry.global_tag);
        }
    }

    fn scope_uses_global_time(reaction_graph: &ReactionGraph, scope: ScopeKey) -> bool {
        reaction_graph.scopes[scope].parent.is_none()
    }

    pub(super) fn scope_ever_active(&self, scope: ScopeKey) -> bool {
        self.scope_ever_active[scope]
    }

    pub(super) fn scope_active(&self, scope: ScopeKey) -> bool {
        self.scope_active[scope]
    }

    fn scope_is_descendant_or_self(
        reaction_graph: &ReactionGraph,
        mut scope: ScopeKey,
        ancestor: ScopeKey,
    ) -> bool {
        loop {
            if scope == ancestor {
                return true;
            }

            let Some(parent) = reaction_graph.scopes[scope].parent else {
                return false;
            };
            scope = parent;
        }
    }
}
