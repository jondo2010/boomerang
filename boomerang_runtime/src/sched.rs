use std::{collections::BinaryHeap, pin::Pin};

use kanal::ReceiveErrorTimeout;
use tinymap::Key as _;

use crate::{
    build_reaction_contexts,
    env::{Enclave, EnclaveKey},
    event::{AsyncEvent, ScheduledActionValue, ScheduledEvent},
    keepalive,
    key_set::KeySetView,
    store::Store,
    CommonContext, Duration, Env, Level, ModeTransitionRequest, ReactionGraph, ReactionKey,
    ReactionSet, ReactionSetLimits, ReactorKey, ScopeKey, SendContext, Tag, TransitionKind,
};

#[derive(Debug)]
struct EventQueue {
    /// Current event queue
    event_queue: BinaryHeap<ScheduledEvent>,
    /// Recycled ReactionSets to avoid allocations
    free_reaction_sets: Vec<ReactionSet>,
    /// Limits for the reaction sets
    reaction_set_limits: ReactionSetLimits,
}

impl EventQueue {
    fn new(reaction_set_limits: ReactionSetLimits) -> Self {
        Self {
            event_queue: BinaryHeap::new(),
            free_reaction_sets: Vec::new(),
            reaction_set_limits,
        }
    }

    /// Push an event into the event queue
    ///
    /// A free event is pulled from the `free_events` vector and then modified with the provided function.
    fn push_event<I>(&mut self, tag: Tag, reactions: I, terminal: bool)
    where
        I: IntoIterator<Item = (Level, ReactionKey)>,
    {
        self.push_event_inner(tag, reactions, terminal, None);
    }

    fn push_action_event<I>(
        &mut self,
        tag: Tag,
        action_value: Option<ScheduledActionValue>,
        reactions: I,
        terminal: bool,
    ) where
        I: IntoIterator<Item = (Level, ReactionKey)>,
    {
        self.push_event_inner(tag, reactions, terminal, action_value);
    }

    fn push_event_inner<I>(
        &mut self,
        tag: Tag,
        reactions: I,
        terminal: bool,
        action_value: Option<ScheduledActionValue>,
    ) where
        I: IntoIterator<Item = (Level, ReactionKey)>,
    {
        let can_merge = self.event_queue.peek().is_some_and(|event| {
            event.tag == tag && (event.action_value.is_none() || action_value.is_none())
        });

        if can_merge {
            // If the tag is the same as the next event, merge the reactions
            let mut event = self.event_queue.peek_mut().unwrap();
            event.reactions.extend_above(reactions);
            event.terminal = event.terminal || terminal;
            if action_value.is_some() {
                event.action_value = action_value;
            }
        } else {
            // Otherwise, push a new event
            let mut reaction_set = self.next_reaction_set();
            reaction_set.extend_above(reactions);
            let event = ScheduledEvent {
                tag,
                reactions: reaction_set,
                terminal,
                action_value,
            };
            self.event_queue.push(event);
        }
    }

    /// Pop the next event from the event queue.
    ///
    /// Any subsequent events with the same tag are merged into the returned event.
    fn pop_next_event(&mut self) -> Option<ScheduledEvent> {
        if let Some(mut event) = self.event_queue.pop() {
            // Merge events with the same tag
            while let Some(next_event) = self.event_queue.peek() {
                if next_event.tag == event.tag {
                    let next_event = self.event_queue.pop().unwrap();
                    event.reactions.merge(&next_event.reactions);
                    event.terminal = event.terminal || next_event.terminal;

                    self.recycle_reaction_set(next_event.reactions);
                } else {
                    break;
                }
            }

            return Some(event);
        }

        None
    }

    /// Get a free [`ReactionSet`] or create a new one if none are available.
    fn next_reaction_set(&mut self) -> ReactionSet {
        self.free_reaction_sets
            .pop()
            .unwrap_or_else(|| ReactionSet::new(&self.reaction_set_limits))
    }

    fn recycle_reaction_set(&mut self, mut reaction_set: ReactionSet) {
        reaction_set.clear();
        self.free_reaction_sets.push(reaction_set);
    }

    /// Peek the tag of the next event in the queue
    fn peek_tag(&self) -> Option<Tag> {
        self.event_queue.peek().map(|event| event.tag)
    }

    /// If the event queue still has events on it, report that.
    fn shutdown(&mut self) {
        if !self.event_queue.is_empty() {
            tracing::warn!(
                "---- There are {} unprocessed future events on the event queue.",
                self.event_queue.len()
            );
            let event = self.event_queue.peek().unwrap();
            tracing::warn!(
                "---- The first future event has timestamp {} after start time.",
                event.tag.offset()
            );
        }
    }

    fn clear(&mut self) {
        while let Some(event) = self.event_queue.pop() {
            self.recycle_reaction_set(event.reactions);
        }
    }

    fn rebase_action_values(
        &mut self,
        store: &mut Pin<Box<Store>>,
        mut map_tag: impl FnMut(Tag) -> Tag,
    ) {
        let mut events = self.event_queue.drain().collect::<Vec<_>>();
        let mut first_move: Option<(crate::ActionKey, Tag, Tag)> = None;
        let mut moves: Option<Vec<(crate::ActionKey, Tag, Tag)>> = None;
        for event in &mut events {
            let new_tag = map_tag(event.tag);
            if let Some(action_value) = &mut event.action_value {
                let action_move = (action_value.key, action_value.stored_tag, new_tag);
                if let Some(moves) = &mut moves {
                    moves.push(action_move);
                } else if let Some(first_move) = first_move.take() {
                    let mut collected = Vec::with_capacity(2);
                    collected.push(first_move);
                    collected.push(action_move);
                    moves = Some(collected);
                } else {
                    first_move = Some(action_move);
                }
                action_value.stored_tag = new_tag;
            }
        }
        if let Some(mut moves) = moves {
            moves.sort_by(|(_, from_a, _), (_, from_b, _)| from_b.cmp(from_a));
            for (action_key, from, to) in moves {
                store.reschedule_action_value(action_key, from, to);
            }
        } else if let Some((action_key, from, to)) = first_move {
            store.reschedule_action_value(action_key, from, to);
        }
        self.event_queue = events.into_iter().collect();
    }
}

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
struct ReadyEvent {
    tag: Tag,
    reactions: ReactionSet,
    terminal: bool,
}

#[derive(Debug)]
struct EventManager {
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
    fn new(
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

    fn push_event<I>(&mut self, tag: Tag, reactions: I, terminal: bool)
    where
        I: IntoIterator<Item = (Level, ReactionKey)>,
    {
        self.root.push_event(tag, reactions, terminal);
    }

    fn push_action_event<I>(
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

    fn peek_tag(&mut self) -> Option<Tag> {
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

    fn pop_next_event(&mut self) -> Option<ReadyEvent> {
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

    fn shutdown(&mut self) {
        self.root.shutdown();
    }

    fn return_reaction_set(&mut self, reaction_set: ReactionSet) {
        if self.has_local_scopes {
            let mut reaction_set = reaction_set;
            reaction_set.clear();
            self.free_reaction_sets.push(reaction_set);
        } else {
            self.root.recycle_reaction_set(reaction_set);
        }
    }

    fn apply_transition(
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

    fn scope_ever_active(&self, scope: ScopeKey) -> bool {
        self.scope_ever_active[scope]
    }

    fn scope_active(&self, scope: ScopeKey) -> bool {
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

#[derive(Debug, Clone)]
pub struct Config {
    /// Whether to skip wall-clock synchronization (execute as fast as possible)
    pub fast_forward: bool,
    /// Whether to keep the scheduler alive for any possible asynchronous events.
    /// If `false`, the scheduler will terminate when there are no more events to process.
    pub keep_alive: bool,
    /// The size of the physical event queue.
    pub physical_event_q_size: usize,
    /// Stop the scheduler after a certain amount of time has passed.
    pub timeout: Option<Duration>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            fast_forward: false,
            keep_alive: false,
            physical_event_q_size: 1024,
            timeout: None,
        }
    }
}

impl Config {
    pub fn with_fast_forward(mut self, fast_forward: bool) -> Self {
        self.fast_forward = fast_forward;
        self
    }

    pub fn with_keep_alive(mut self, keep_alive: bool) -> Self {
        self.keep_alive = keep_alive;
        self
    }

    /// Set the capacity of the physical event queue.
    ///
    /// If the queue is full, this call will block until there is space available.
    pub fn with_queue_size(mut self, physical_event_q_size: usize) -> Self {
        self.physical_event_q_size = physical_event_q_size;
        self
    }

    /// Set a timeout for the scheduler.
    /// The scheduler will terminate after the given duration has passed.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }
}

#[derive(Debug)]
struct LogicalTimeBarrier {
    /// The last released tag
    released_tag: Tag,
    provisional_tag: Tag,
    /// The send context for the upstream enclave
    upstream_ctx: SendContext,
    /// Optional delay for the upstream connection
    upstream_delay: Option<Duration>,
}

impl LogicalTimeBarrier {
    #[tracing::instrument(skip(self), fields(tag = %tag, released = %self.released_tag))]
    pub fn release_tag(&mut self, tag: Tag) {
        tracing::trace!("Release");

        if tag < self.released_tag {
            tracing::warn!(
                "Cannot release a tag ({tag}) earlier than the last released tag {}",
                self.released_tag
            );
        }
        self.released_tag = tag;
        // Reset the provisional tag
        self.provisional_tag = Tag::NEVER;
    }

    pub fn release_tag_provisional(&mut self, tag: Tag) {
        if tag <= self.provisional_tag {
            self.release_tag(tag);
        }
    }

    #[inline]
    /// Try to acquire the given tag without blocking.
    pub fn try_acquire_tag(&mut self, tag: Tag) -> bool {
        tag <= self.released_tag
    }

    /// Acquire the given tag, blocking until it is released, or an [`AsyncEvent`] is received.
    ///
    /// If an async event is received, it is returned to the caller. A return value of `None` indicates that the tag has been released.
    #[inline]
    #[tracing::instrument(skip(self, tag, this_enclave, event_rx), fields(tag = %tag))]
    pub fn acquire_tag(
        &mut self,
        tag: Tag,
        this_enclave: EnclaveKey,
        event_rx: &crate::Receiver<AsyncEvent>,
    ) -> Option<AsyncEvent> {
        // Since this is a delayed connection, we can go back in time and need to
        // acquire the latest upstream tag that can create an event at the given
        // tag.
        let upstream_tag = if let Some(delay) = self.upstream_delay {
            tag.pre(delay)
        } else {
            tag
        };

        tracing::trace!(upstream_tag = %upstream_tag, "Try acquire");
        if self.try_acquire_tag(upstream_tag) {
            return None;
        }

        tracing::trace!(%upstream_tag, "Releasing provisional tag");
        self.provisional_tag = upstream_tag;
        if !self
            .upstream_ctx
            .release_provisional(this_enclave, upstream_tag)
        {
            // The upstream has terminated try to return a queued event here. If the upstream terminated, we probably
            // have an event queued from it. This prevents pre-mature termination of this enclave.
            tracing::warn!("Upstream has terminated");
            return event_rx.try_recv().expect("Upstream terminated");
        }

        // Block until the tag is released
        tracing::trace!("Blocking");
        event_rx.recv().ok()
    }
}

#[derive(Debug, Default)]
pub struct Stats {
    /// Number of `tag`s processed
    processed_tags: usize,
    /// Number of reactions processed
    processed_reactions: usize,
    /// Number of scheduled async events
    processed_events: usize,
    /// Number of ports set
    set_ports: usize,
    /// Number of scheduled, sync actions
    scheduled_actions: usize,
}

impl Stats {
    pub fn increment_processed_tags(&mut self) {
        self.processed_tags += 1;
    }
    pub fn increment_processed_reactions(&mut self, count: usize) {
        self.processed_reactions += count;
    }
    pub fn increment_processed_events(&mut self) {
        self.processed_events += 1;
    }
    pub fn increment_set_ports(&mut self) {
        self.set_ports += 1;
    }
    pub fn increment_scheduled_actions(&mut self, count: usize) {
        self.scheduled_actions += count;
    }
}

impl std::fmt::Display for Stats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Stats")
            .field("Processed tags", &self.processed_tags)
            .field("Processed reactions", &self.processed_reactions)
            .field("Processed events", &self.processed_events)
            .field("Set ports", &self.set_ports)
            .field("Scheduled actions", &self.scheduled_actions)
            .finish()
    }
}

#[derive(Debug)]
pub struct Scheduler {
    /// The enclave key
    key: EnclaveKey,
    /// The scheduler config
    config: Config,
    /// The reactor runtime store
    store: Pin<Box<Store>>,
    /// The reaction graph containing all static dependency and relationship information
    reaction_graph: ReactionGraph,
    /// Asynchronous events receiver
    event_rx: crate::Receiver<AsyncEvent>,
    /// Event queues for root-scope and mode-local events.
    events: EventManager,
    /// Initial physical time.
    start_time: std::time::Instant,
    /// Current tag
    current_tag: Tag,
    /// A shutdown has been scheduled at this time.
    shutdown_tag: Option<Tag>,
    /// Shutdown channel
    shutdown_tx: keepalive::Sender,
    /// Logical time barriers for each upstream enclave
    upstream_enclaves: tinymap::TinySecondaryMap<EnclaveKey, LogicalTimeBarrier>,
    /// The senders for downstream enclaves
    downstream_enclaves: tinymap::TinySecondaryMap<EnclaveKey, SendContext>,
    /// Runtime statistics
    stats: Stats,
    /// Reusable buffer for reaction keys to avoid allocations in hot loops
    reaction_buffer: Vec<ReactionKey>,
    /// Reusable buffer for mode transitions to avoid allocations in hot loops
    transition_buffer: Vec<(ReactorKey, ModeTransitionRequest)>,
    /// Whether this graph contains any modes and needs modal scope checks in the hot path.
    has_modes: bool,
}

impl Scheduler {
    /// Create a new Scheduler instance.
    ///
    /// The Scheduler will be initialized with the provided environment and reaction graph.
    ///
    /// # Arguments
    ///
    /// * `env` - The environment containing all the runtime data structures.
    /// * `reaction_graph` - The reaction graph containing all static dependency and relationship information.
    pub fn new(key: EnclaveKey, enclave: Enclave, config: Config) -> Self {
        let Enclave {
            env,
            graph,
            event_tx,
            event_rx,
            downstream_enclaves,
            upstream_enclaves,
            shutdown_tx,
            shutdown_rx,
        } = enclave;

        let start_time = std::time::Instant::now();
        let reaction_capacity = env.reactions.len();

        // Find the maximum level in the reaction graph
        let max_level = graph
            .action_triggers
            .values()
            .chain(graph.port_triggers.values())
            .flat_map(|level_reactions| level_reactions.iter().map(|(level, _)| level))
            .max()
            .copied()
            .unwrap_or_default();

        let reaction_set_limits = ReactionSetLimits {
            max_level,
            num_keys: env.reactions.len(),
        };
        // Build contexts for each reaction
        let contexts = build_reaction_contexts(key, &graph, start_time, event_tx, shutdown_rx);

        let store = Store::new(env, contexts, &graph);
        let has_modes = !graph.modes.is_empty();
        let events = EventManager::new(reaction_set_limits, &graph, &store);

        let upstream_enclaves = upstream_enclaves
            .into_iter()
            .map(|(enclave_key, upstream_ref)| {
                (
                    enclave_key,
                    LogicalTimeBarrier {
                        released_tag: Tag::NEVER,
                        provisional_tag: Tag::NEVER,
                        upstream_ctx: upstream_ref.send_ctx,
                        upstream_delay: upstream_ref.delay,
                    },
                )
            })
            .collect();

        let downstream_enclaves = downstream_enclaves
            .into_iter()
            .map(|(enclave_key, downstream_ref)| (enclave_key, downstream_ref.send_ctx))
            .collect();

        Self {
            key,
            config,
            store,
            reaction_graph: graph,
            event_rx,
            events,
            start_time,
            current_tag: Tag::NEVER,
            shutdown_tag: None,
            shutdown_tx,
            upstream_enclaves,
            downstream_enclaves,
            stats: Stats::default(),
            reaction_buffer: Vec::with_capacity(reaction_capacity),
            transition_buffer: Vec::with_capacity(reaction_capacity),
            has_modes,
        }
    }

    /// Handle an asynchronous event from the event queue
    #[tracing::instrument(skip(self, ), fields(event = %event))]
    fn handle_async_event(&mut self, event: AsyncEvent) {
        self.stats.increment_processed_events();
        tracing::trace!("Handling");
        match event {
            AsyncEvent::TagRelease { enclave, tag } => {
                self.upstream_enclaves
                    .get_mut(enclave)
                    .expect("Unknown upstream enclave")
                    .release_tag(tag);
            }
            AsyncEvent::TagReleaseProvisional { enclave, tag } => {
                if tag <= self.current_tag {
                    if tag < self.current_tag {
                        tracing::warn!(tag = %tag, "Ignoring empty event in the past");
                    }
                    return;
                }
                // TagReleaseProvisional events are coming from downstream enclaves.
                // If this enclave is also an upstream (cycle), then also release it provisionally.
                if let Some(barrier) = self.upstream_enclaves.get_mut(enclave) {
                    barrier.release_tag_provisional(tag);
                }
                self.events.push_event(tag, std::iter::empty(), false);
            }
            AsyncEvent::Logical { tag, key, value } => {
                if tag <= self.current_tag {
                    tracing::warn!(tag = %tag, "Ignoring empty event in the past");
                    return;
                }
                let downstream = self.reaction_graph.action_triggers[key].iter().copied();
                self.store.push_action_value(key, tag, value);
                self.events
                    .push_action_event(key, tag, downstream, false, &self.reaction_graph);
            }
            AsyncEvent::Physical { time, key, value } => {
                let tag = Tag::from_physical_time(self.start_time, time);
                let downstream = self.reaction_graph.action_triggers[key].iter().copied();
                self.store.push_action_value(key, tag, value);
                self.events
                    .push_action_event(key, tag, downstream, false, &self.reaction_graph);
            }
            AsyncEvent::Shutdown { delay } => {
                let tag = self.current_tag.delay(delay);
                self.schedule_shutdown_at(tag);
            }
        }
    }

    fn schedule_shutdown_at(&mut self, tag: Tag) {
        let shutdown_reactions = &self
            .reaction_graph
            .modal_schedule_index
            .all_shutdown_reactions;

        for &action_key in &self
            .reaction_graph
            .modal_schedule_index
            .all_shutdown_actions_unique
        {
            self.store.push_action_value(action_key, tag, Box::new(()));
        }

        self.events.push_event(
            tag,
            shutdown_reactions.iter().map(|reaction| reaction.reaction),
            true,
        );
    }

    /// Execute startup of the Scheduler.
    #[tracing::instrument(skip(self))]
    pub fn startup(&mut self) {
        let tag = Tag::ZERO;

        // Initialize the event queue with the startup actions
        for &(action_key, tag) in &self.reaction_graph.startup_actions {
            self.store.push_action_value(action_key, tag, Box::new(()));
            let downstream = self.reaction_graph.action_triggers[action_key]
                .iter()
                .inspect(|(lvl, reaction_key)| {
                    tracing::trace!(level = %lvl, reaction = %reaction_key, tag = %tag, "Startup reaction");
                })
                .copied();
            self.events
                .push_action_event(action_key, tag, downstream, false, &self.reaction_graph);
        }

        // Schedule a shutdown event if a timeout is set
        if let Some(timeout) = self.config.timeout {
            let tag = tag.delay(timeout);
            tracing::info!(tag = %tag, "Timeout set, scheduling shutdown");
            self.schedule_shutdown_at(tag);
        }

        tracing::info!(tag = %tag, "Starting the execution.");

        self.current_tag = tag.decrement();

        // Release the current tag to downstream reactors
        self.release_tag_downstream(self.current_tag);

        self.start_time = std::time::Instant::now();
    }

    /// Final shutdown of the Scheduler. The last tag has already been processed.
    #[tracing::instrument(skip(self))]
    fn shutdown(&mut self) {
        tracing::info!("Shutting down.");

        self.events.shutdown();

        let logical_elapsed = self.shutdown_tag.unwrap().offset();
        tracing::info!("---- Elapsed logical time: {logical_elapsed}",);
        // If physical_start_time is 0, then execution didn't get far enough along to initialize this.
        let physical_elapsed = std::time::Instant::now() - self.start_time;
        tracing::info!("---- Elapsed physical time: {physical_elapsed:?}");

        tracing::info!(stats = ?self.stats, "Scheduler has been shut down.");
    }

    /// Try to receive an asynchronous event
    #[tracing::instrument(skip(self))]
    fn receive_event_async(&mut self) -> Option<AsyncEvent> {
        if let Some(shutdown) = self.shutdown_tag {
            let abs = shutdown.to_logical_time(self.start_time);
            if let Some(timeout) = abs.checked_duration_since(std::time::Instant::now()) {
                tracing::debug!(timeout = ?timeout, "Waiting for async event.");
                self.event_rx.recv_timeout(timeout).ok()
            } else {
                tracing::debug!("Cannot wait, already past programmed shutdown time...");
                None
            }
        } else if self.config.keep_alive {
            tracing::debug!("Waiting indefinitely for async event.");
            self.event_rx.recv().ok()
        } else {
            None
        }
    }

    /// Release the current tag to downstream reactors
    #[tracing::instrument(skip(self, current_tag), fields(tag = %current_tag))]
    fn release_tag_downstream(&self, current_tag: Tag) {
        for (key, ctx) in self.downstream_enclaves.iter() {
            let event = AsyncEvent::release(self.key, current_tag);
            tracing::trace!(downstream = %key, event = %event, "Releasing downstream");
            if !ctx.schedule_external(event) && self.shutdown_tag.is_none() {
                tracing::warn!(
                    "Failed to send tag downstream, downstream has unexpectedly terminated."
                );
            }
        }
    }

    #[tracing::instrument(skip(self), fields(tag = %self.current_tag))]
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> bool {
        // Pump the event queue
        while let Ok(Some(async_event)) = self.event_rx.try_recv() {
            self.handle_async_event(async_event);
        }

        if let Some(next_tag) = self.events.peek_tag() {
            tracing::trace!(next_tag = %next_tag, "Trying next tag");

            // Wait until all upstream barriers are released
            for (_upstream_enclave_key, barrier) in self.upstream_enclaves.iter_mut() {
                if let Some(async_event) = barrier.acquire_tag(next_tag, self.key, &self.event_rx) {
                    self.handle_async_event(async_event);
                    // Returned early due to async event
                    return true;
                }
            }

            if !self.config.fast_forward {
                let target = next_tag.to_logical_time(self.start_time);
                if self.synchronize_wall_clock(target) {
                    // Woken up by async event
                    return true;
                }
            }

            let mut event = self.events.pop_next_event().unwrap();

            tracing::debug!(event = ?event, "Processing");

            if event.terminal {
                // Signal to any waiting threads that the scheduler is shutting down.
                self.shutdown_tx.shutdown();
            }

            self.process_tag(event.tag, event.reactions.view(), event.terminal);

            self.current_tag = event.tag;

            // Return the ReactionSet to the free pool
            self.events.return_reaction_set(event.reactions);

            // Release the current tag to downstream reactors
            self.release_tag_downstream(self.current_tag);

            self.stats.increment_processed_tags();

            if event.terminal {
                // Break out of the event loop;
                self.shutdown_tag = Some(self.current_tag);
                return false;
            }
        } else if let Some(async_event) = self.receive_event_async() {
            self.handle_async_event(async_event);
        } else {
            tracing::debug!("No more events in queue, pushing a shutdown event.");
            // Shutdown event will be processed at the next event loop iteration
            let shutdown = self.current_tag.delay(Duration::ZERO);
            self.shutdown_tag = Some(shutdown);
            self.schedule_shutdown_at(shutdown);
        }

        true
    }

    #[tracing::instrument(skip(self), fields(key = %self.key))]
    pub fn event_loop(&mut self) {
        self.startup();

        while self.next() {}

        self.shutdown();
    }

    // Wait until the wall-clock time is reached
    #[tracing::instrument(skip(self, target))]
    fn synchronize_wall_clock(&mut self, target: std::time::Instant) -> bool {
        let now = std::time::Instant::now();

        match now.cmp(&target) {
            std::cmp::Ordering::Less => {
                let advance = target - now;
                tracing::trace!(advance = ?advance, "Need to sleep");

                match self.event_rx.recv_timeout(advance) {
                    Ok(event) => {
                        tracing::debug!(event = %event, "Sleep interrupted by");
                        self.handle_async_event(event);
                        return true;
                    }
                    Err(ReceiveErrorTimeout::Closed) | Err(ReceiveErrorTimeout::SendClosed) => {
                        let remaining = target.checked_duration_since(std::time::Instant::now());
                        if let Some(remaining) = remaining {
                            tracing::debug!(remaining = ?remaining,
                                "Sleep interrupted disconnect, sleeping for remaining",
                            );
                            std::thread::sleep(remaining);
                        }
                    }
                    Err(ReceiveErrorTimeout::Timeout) => {}
                }
            }

            std::cmp::Ordering::Greater => {
                let delay = now - target;
                tracing::warn!(delay = ?delay, "running late");
            }

            std::cmp::Ordering::Equal => {}
        }

        false
    }

    /// Process the reactions at this tag in increasing order of level.
    ///
    /// Reactions at a level N may trigger further reactions at levels M>N
    #[tracing::instrument(skip(self, reaction_view), fields(tag = %tag))]
    pub fn process_tag(
        &mut self,
        tag: Tag,
        reaction_view: KeySetView<ReactionKey>,
        terminal: bool,
    ) {
        self.transition_buffer.clear();
        reaction_view.for_each_level(|level, reaction_keys, next_levels| {
            tracing::trace!(level=?level, "Iter");

            self.reaction_buffer.clear();
            if self.has_modes {
                for reaction_key in reaction_keys {
                    if self.reaction_is_enabled_at_current_tag(reaction_key, terminal) {
                        self.reaction_buffer.push(reaction_key);
                    }
                }
            } else {
                self.reaction_buffer.extend(reaction_keys);
            }

            self.stats
                .increment_processed_reactions(self.reaction_buffer.len());

            // Safety: reaction_keys in the same level are guaranteed to be independent of each other.
            let iter_ctx = unsafe {
                self.store
                    .iter_borrow_storage(self.reaction_buffer.iter().copied())
            }
            .enumerate();

            #[cfg(feature = "parallel")]
            use rayon::prelude::ParallelIterator;

            #[cfg(feature = "parallel")]
            let iter_ctx = rayon::prelude::ParallelBridge::par_bridge(iter_ctx);

            let iter_ctx_res = iter_ctx.map(|(idx, trigger_ctx)| (idx, trigger_ctx.trigger(tag)));

            #[cfg(feature = "parallel")]
            let iter_ctx_res = iter_ctx_res.collect::<Vec<_>>();

            let mut pending_shutdown_tag = None;
            for (idx, trigger_res) in iter_ctx_res {
                let reaction_key = self.reaction_buffer[idx];
                let reactor_key = self.reaction_graph.reaction_reactors[reaction_key];
                if let Some(request) = &trigger_res.scheduled_mode {
                    if let Some((_, existing)) = self
                        .transition_buffer
                        .iter_mut()
                        .find(|(existing_reactor, _)| *existing_reactor == reactor_key)
                    {
                        *existing = request.clone();
                    } else {
                        self.transition_buffer.push((reactor_key, request.clone()));
                    }
                }

                if let Some(shutdown_tag) = trigger_res.scheduled_shutdown {
                    // if the new shutdown tag is earlier than the current shutdown tag, update the shutdown tag and
                    // schedule a shutdown event
                    if self.shutdown_tag.map(|t| shutdown_tag < t).unwrap_or(true) {
                        self.shutdown_tag = Some(shutdown_tag);
                        pending_shutdown_tag = Some(shutdown_tag);
                    }
                }

                // Submit events to the event queue for all scheduled actions
                self.stats
                    .increment_scheduled_actions(trigger_res.scheduled_actions.len());
                for &(action_key, tag) in trigger_res.scheduled_actions.iter() {
                    let downstream = self.reaction_graph.action_triggers[action_key]
                        .iter()
                        .copied();
                    self.events.push_action_event(
                        action_key,
                        tag,
                        downstream,
                        false,
                        &self.reaction_graph,
                    );
                }
            }

            if let Some(shutdown_tag) = pending_shutdown_tag {
                self.schedule_shutdown_at(shutdown_tag);
            }

            // Collect all the reactions that are triggered by the ports
            if let Some(mut next_levels) = next_levels {
                let reaction_graph = &self.reaction_graph;
                let events = &self.events;
                let has_modes = self.has_modes;

                for port_key in self.store.iter_set_port_keys() {
                    self.stats.increment_set_ports();
                    let downstream = reaction_graph.port_triggers[port_key].iter().copied();
                    if has_modes {
                        next_levels.extend_above(downstream.filter(|&(_, reaction_key)| {
                            let scope_key = reaction_graph.reaction_scopes[reaction_key];
                            events.scope_active(scope_key)
                        }));
                    } else {
                        next_levels.extend_above(downstream);
                    }
                }
            }
        });

        if self.transition_buffer.is_empty() {
            self.store.reset_ports();
            return;
        }

        for idx in 0..self.transition_buffer.len() {
            let (reactor_key, request) = self.transition_buffer[idx].clone();
            self.events.apply_transition(
                reactor_key,
                &request,
                &mut self.store,
                &self.reaction_graph,
                tag,
            );
        }
        self.transition_buffer.clear();

        self.store.reset_ports();
    }

    fn reaction_is_enabled_at_current_tag(
        &self,
        reaction_key: ReactionKey,
        terminal: bool,
    ) -> bool {
        debug_assert!(self.has_modes);

        let scope_key = self.reaction_graph.reaction_scopes[reaction_key];
        let shutdown_lifecycle = terminal && self.reaction_graph.is_shutdown_reaction(reaction_key);
        if shutdown_lifecycle {
            return self.events.scope_ever_active(scope_key);
        }

        if !self.events.scope_active(scope_key) {
            return false;
        }

        debug_assert!(
            self.reaction_graph.reaction_modes[reaction_key]
                .as_ref()
                .is_none_or(|filter| {
                    self.reaction_graph.scopes[scope_key]
                        .mode
                        .is_some_and(|mode| {
                            let modes = filter.modes();
                            modes.len() == 1 && modes[0] == mode
                        })
                }),
            "reaction mode filters are expected to be equivalent to the static reaction scope"
        );

        true
    }

    /// Consume the scheduler and return the `Env` instance.
    ///
    /// This method is useful for testing purposes, as it allows the caller to inspect reactor states after the
    /// scheduler has been run.
    pub fn into_env(self) -> Env {
        self.store.into_env()
    }
}

/// Execute the given enclaves with the provided configuration.
///
/// This function will create a new `Scheduler` thread for each enclave and run its event loop.
///
/// # Arguments
///
/// * `enclaves` - An iterator over the enclaves to be executed.
/// * `config` - The configuration to be used for the schedulers.
///
/// # Returns
///
/// A vector of `Env` instances, one for each executed enclave.
///
/// # Panics
///
/// Panics if there is an error during the execution of any enclave.
pub fn execute_enclaves(
    #[allow(unused_mut)] mut enclaves: impl Iterator<Item = (EnclaveKey, Enclave)> + Send,
    config: Config,
) -> tinymap::TinySecondaryMap<EnclaveKey, Env> {
    let handles: Vec<_> = enclaves
        .filter_map(move |(enclave_key, enclave)| {
            if enclave.env.reactions.is_empty() {
                // If there are no reactions, there is nothing to do
                tracing::info!("No reactions to execute for enclave {enclave_key:?}");
                None
            } else {
                tracing::info!("Starting scheduler for enclave {enclave_key:?}");
                Some(Scheduler::new(enclave_key, enclave, config.clone()))
            }
        })
        .map(|mut sched| {
            std::thread::Builder::new()
                .name(sched.key.to_string())
                .spawn(move || {
                    sched.event_loop();
                    (sched.key, sched.into_env())
                })
                .unwrap()
        })
        .collect();

    handles
        .into_iter()
        .map(|handle| handle.join().expect("Thread panicked"))
        .collect()
}
