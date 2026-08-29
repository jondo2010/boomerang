use std::collections::BinaryHeap;

use super::{
    queue::{EventQueue, ScheduledActionValue},
    ExecutionStorage, ModeTransition, ScheduleAccess,
};
use crate::{key_set::KeySet, Duration, Level, ReactionSetLimits, Tag, TransitionKind};

/// Clock state for a scheduler scope that may be suspended and resumed by modal transitions.
#[derive(Debug)]
struct ScopeClockState {
    /// Global scheduler tag at which the scope most recently became active.
    activation_global: Tag,
    /// Scope-local tag that corresponds to [`activation_global`](Self::activation_global).
    activation_local: Tag,
    /// Whether events exactly at the activation tag are allowed to run in this activation.
    allow_activation_tag: bool,
    /// Scope-local tag captured when the scope most recently became inactive.
    suspended_local: Tag,
    /// Generation counter used to invalidate stale frontier heap entries for this scope.
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

/// Heap entry for the next runnable event in a scope-local queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ScopeFrontierEntry<S: tinymap::Key> {
    /// Global tag corresponding to the scope queue's current local front event.
    global_tag: Tag,
    /// Scope whose queue contributed this frontier entry.
    scope: S,
    /// Clock generation observed when this entry was pushed.
    epoch: u64,
}

impl<S: tinymap::Key> Ord for ScopeFrontierEntry<S> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.global_tag
            .cmp(&other.global_tag)
            .then(self.scope.index().cmp(&other.scope.index()))
            .reverse()
    }
}

impl<S: tinymap::Key> PartialOrd for ScopeFrontierEntry<S> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Event returned to the scheduler after root and scope-local queues are merged at one tag.
#[derive(Debug)]
pub(super) struct ReadyEvent<R: tinymap::Key> {
    /// Global tag at which the contained reactions are ready.
    pub(super) tag: Tag,
    /// Reactions ready to execute at [`tag`](Self::tag).
    pub(super) reactions: KeySet<R>,
    /// Whether this event indicates scheduler termination.
    pub(super) terminal: bool,
}

/// Owns root and scope-local event queues for modal scheduling.
#[derive(Debug)]
pub(super) struct EventManager<S: ScheduleAccess> {
    /// Global event queue used for root-scoped work and non-modal fast paths.
    root: EventQueue<S::Reaction, S::Action>,
    /// Canonical scope keys retained so modal updates never borrow mutable storage through the schedule.
    scopes: Vec<S::Scope>,
    /// Current active/inactive state for each static scope.
    scope_active: tinymap::TinySecondaryMap<S::Scope, bool>,
    /// Whether each scope has ever been active during this scheduler run.
    scope_ever_active: tinymap::TinySecondaryMap<S::Scope, bool>,
    /// Whether scope-local startup reactions have already fired for each scope.
    scope_startup_fired: tinymap::TinySecondaryMap<S::Scope, bool>,
    /// Current mode per reactor for modal scheduling decisions.
    reactor_modes: tinymap::TinySecondaryMap<S::Reactor, Option<S::Mode>>,
    /// Local clock state for each static scope.
    scope_clocks: tinymap::TinySecondaryMap<S::Scope, ScopeClockState>,
    /// Per-scope queues for events scheduled in scope-local time.
    scope_queues: tinymap::TinySecondaryMap<S::Scope, EventQueue<S::Reaction, S::Action>>,
    /// Min-heap of each active scope's next event, ordered by global tag.
    frontier: BinaryHeap<ScopeFrontierEntry<S::Scope>>,
    /// Reusable reaction sets for merged ready events.
    free_reaction_sets: Vec<KeySet<S::Reaction>>,
    /// Key and level limits used when allocating reaction sets.
    reaction_set_limits: ReactionSetLimits,
    /// Whether this graph has any modal scopes requiring local queues.
    has_local_scopes: bool,
}

impl<S: ScheduleAccess> EventManager<S> {
    pub(super) fn new(reaction_set_limits: ReactionSetLimits, schedule: &S) -> Self {
        let root = EventQueue::new(reaction_set_limits.clone());
        let mut scope_active = tinymap::TinySecondaryMap::new();
        let mut scope_ever_active = tinymap::TinySecondaryMap::new();
        let mut scope_startup_fired = tinymap::TinySecondaryMap::new();
        let reactor_modes = schedule.reactor_initial_modes().collect();
        let mut scope_clocks = tinymap::TinySecondaryMap::new();
        let mut scope_queues = tinymap::TinySecondaryMap::new();

        let scopes = schedule.scopes().collect::<Vec<_>>();
        for &scope in &scopes {
            let active = Self::scope_is_active_with_modes(schedule, &reactor_modes, scope);
            scope_active.insert(scope, active);
            scope_ever_active.insert(scope, active);
            scope_startup_fired.insert(scope, active);
            scope_clocks.insert(scope, ScopeClockState::new(active));
            scope_queues.insert(scope, EventQueue::new(reaction_set_limits.clone()));
        }

        Self {
            root,
            scopes,
            scope_active,
            scope_ever_active,
            scope_startup_fired,
            reactor_modes,
            scope_clocks,
            scope_queues,
            frontier: BinaryHeap::new(),
            free_reaction_sets: Vec::new(),
            reaction_set_limits,
            has_local_scopes: schedule.has_modes(),
        }
    }

    pub(super) fn push_event<I>(&mut self, tag: Tag, reactions: I, terminal: bool)
    where
        I: IntoIterator<Item = (Level, S::Reaction)>,
    {
        self.root.push_event(tag, reactions, terminal);
    }

    pub(super) fn push_action_event<I>(
        &mut self,
        action_key: S::Action,
        tag: Tag,
        reactions: I,
        terminal: bool,
        schedule: &S,
    ) where
        I: IntoIterator<Item = (Level, S::Reaction)>,
    {
        let action_value = ScheduledActionValue {
            key: action_key,
            stored_tag: tag,
        };
        if !self.has_local_scopes {
            self.root.push_action_event(tag, None, reactions, terminal);
            return;
        }

        let scope = schedule.action_scope(action_key);
        if !schedule.action_is_logical(action_key) || Self::scope_uses_global_time(schedule, scope)
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
        scope: S::Scope,
        local_tag: Tag,
        action_value: ScheduledActionValue<S::Action>,
        reactions: I,
        terminal: bool,
        schedule: &S,
    ) where
        I: IntoIterator<Item = (Level, S::Reaction)>,
    {
        if Self::scope_uses_global_time(schedule, scope) {
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

    pub(super) fn pop_next_event(&mut self) -> Option<ReadyEvent<S::Reaction>> {
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

    pub(super) fn return_reaction_set(&mut self, reaction_set: KeySet<S::Reaction>) {
        if self.has_local_scopes {
            let mut reaction_set = reaction_set;
            reaction_set.clear();
            self.free_reaction_sets.push(reaction_set);
        } else {
            self.root.recycle_reaction_set(reaction_set);
        }
    }

    pub(super) fn apply_transition<E: ExecutionStorage<S>>(
        &mut self,
        reactor_key: S::Reactor,
        request: &ModeTransition<S::Mode>,
        schedule: &S,
        storage: &mut E,
        current_tag: Tag,
    ) {
        let target_scope = schedule.mode_scope(request.target);

        if matches!(request.transition, TransitionKind::Reset) {
            self.reset_scope_subtree(target_scope, schedule, storage);
            self.reset_child_modes_in_scope(schedule, target_scope);
        }

        self.set_mode(reactor_key, request.target);
        let startup_scopes = self.sync_active_scopes(
            schedule,
            storage,
            current_tag,
            target_scope,
            request.transition,
        );
        self.schedule_startup_reactions(
            &startup_scopes,
            schedule,
            storage,
            current_tag.delay(Duration::ZERO),
        );

        if matches!(request.transition, TransitionKind::Reset) {
            self.schedule_reset_timer_startups(target_scope, schedule, storage);
            self.schedule_reset_reactions(
                target_scope,
                schedule,
                current_tag.delay(Duration::ZERO),
            );
        }
    }

    fn reset_scope_subtree<E: ExecutionStorage<S>>(
        &mut self,
        root_scope: S::Scope,
        schedule: &S,
        storage: &mut E,
    ) {
        for scope in schedule.scope_descendants(root_scope) {
            self.scope_queues[scope].clear();
            let clock = &mut self.scope_clocks[scope];
            clock.suspended_local = Tag::ZERO;
            clock.activation_local = Tag::ZERO;
            clock.frontier_epoch = clock.frontier_epoch.wrapping_add(1);
        }

        for index in 0..schedule.scope_logical_action_count(root_scope) {
            let action_key = schedule.scope_logical_action(root_scope, index);
            storage.clear_action_values(action_key);
        }
    }

    fn sync_active_scopes<E: ExecutionStorage<S>>(
        &mut self,
        schedule: &S,
        storage: &mut E,
        current_tag: Tag,
        reset_root: S::Scope,
        transition: TransitionKind,
    ) -> Vec<S::Scope> {
        let activation_global = current_tag;
        let mut startup_scopes = Vec::new();

        for scope_index in 0..self.scopes.len() {
            let scope = self.scopes[scope_index];
            let new_active = self.scope_is_active(schedule, scope);
            let reset = matches!(transition, TransitionKind::Reset)
                && Self::scope_is_descendant_or_self(schedule, scope, reset_root);

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
                    self.scope_queues[scope].rebase_action_values(
                        |action, from, to| storage.reschedule_action_value(action, from, to),
                        |local_tag| {
                            local_to_global(
                                activation_global,
                                activation_local,
                                allow_activation_tag,
                                local_tag,
                            )
                        },
                    );
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

    fn schedule_startup_reactions<E: ExecutionStorage<S>>(
        &mut self,
        scopes: &[S::Scope],
        schedule: &S,
        storage: &mut E,
        tag: Tag,
    ) {
        if scopes.is_empty() {
            return;
        }

        let has_startup_reactions = scopes
            .iter()
            .any(|&scope| schedule.scope_startup_count(scope) != 0);
        if !has_startup_reactions {
            return;
        }

        for &scope in scopes {
            for index in 0..schedule.scope_startup_count(scope) {
                let (action, _) = schedule.scope_startup(scope, index);
                storage.push_action_value(action, tag, Box::new(()));
            }
        }

        for &scope in scopes {
            self.push_event(
                tag,
                (0..schedule.scope_startup_count(scope))
                    .map(|index| schedule.scope_startup(scope, index).1),
                false,
            );
        }
    }

    fn schedule_reset_timer_startups<E: ExecutionStorage<S>>(
        &mut self,
        root_scope: S::Scope,
        schedule: &S,
        storage: &mut E,
    ) {
        for index in 0..schedule.scope_timer_startup_count(root_scope) {
            let (action_key, local_tag) = schedule.scope_timer_startup(root_scope, index);
            let scope = schedule.action_scope(action_key);
            let global_tag = if Self::scope_uses_global_time(schedule, scope) {
                local_tag
            } else {
                self.scope_clocks[scope].local_to_global(local_tag)
            };
            storage.push_action_value(action_key, global_tag, Box::new(()));
            self.push_local_action_event(
                scope,
                local_tag,
                ScheduledActionValue {
                    key: action_key,
                    stored_tag: global_tag,
                },
                schedule.action_triggers(action_key),
                false,
                schedule,
            );
        }
    }

    fn schedule_reset_reactions(&mut self, root_scope: S::Scope, schedule: &S, tag: Tag) {
        let mut reset_reactions = schedule.scope_reset_reactions(root_scope).peekable();
        if reset_reactions.peek().is_some() {
            self.push_event(tag, reset_reactions, false);
        }
    }

    fn next_reaction_set(&mut self) -> KeySet<S::Reaction> {
        self.free_reaction_sets
            .pop()
            .unwrap_or_else(|| KeySet::new(&self.reaction_set_limits))
    }

    fn refresh_frontier(&mut self, scope: S::Scope) {
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

    fn scope_uses_global_time(schedule: &S, scope: S::Scope) -> bool {
        schedule.scope_parent(scope).is_none()
    }

    fn current_mode(&self, reactor_key: S::Reactor) -> Option<S::Mode> {
        self.reactor_modes.get(reactor_key).copied().flatten()
    }

    fn set_mode(&mut self, reactor_key: S::Reactor, mode: S::Mode) {
        self.reactor_modes.insert(reactor_key, Some(mode));
    }

    fn reset_child_modes_in_scope(&mut self, schedule: &S, scope: S::Scope) {
        let reactor_modes = schedule
            .reactor_root_scopes()
            .filter(|&(_, root_scope)| {
                root_scope != scope
                    && Self::scope_is_descendant_or_self(schedule, root_scope, scope)
            })
            .map(|(reactor_key, _)| (reactor_key, schedule.reactor_initial_mode(reactor_key)))
            .collect::<Vec<_>>();

        for (reactor_key, initial_mode) in reactor_modes {
            self.reactor_modes.insert(reactor_key, initial_mode);
        }
    }

    fn scope_is_active(&self, schedule: &S, mut scope_key: S::Scope) -> bool {
        loop {
            if let Some(mode_key) = schedule.scope_mode(scope_key) {
                if self.current_mode(schedule.scope_reactor(scope_key)) != Some(mode_key) {
                    return false;
                }
            }

            let Some(parent) = schedule.scope_parent(scope_key) else {
                return true;
            };
            scope_key = parent;
        }
    }

    fn scope_is_active_with_modes(
        schedule: &S,
        reactor_modes: &tinymap::TinySecondaryMap<S::Reactor, Option<S::Mode>>,
        mut scope_key: S::Scope,
    ) -> bool {
        loop {
            if let Some(mode_key) = schedule.scope_mode(scope_key) {
                if reactor_modes
                    .get(schedule.scope_reactor(scope_key))
                    .copied()
                    .flatten()
                    != Some(mode_key)
                {
                    return false;
                }
            }

            let Some(parent) = schedule.scope_parent(scope_key) else {
                return true;
            };
            scope_key = parent;
        }
    }

    pub(super) fn scope_ever_active(&self, scope: S::Scope) -> bool {
        self.scope_ever_active[scope]
    }

    pub(super) fn scope_active(&self, scope: S::Scope) -> bool {
        self.scope_active[scope]
    }

    fn scope_is_descendant_or_self(schedule: &S, mut scope: S::Scope, ancestor: S::Scope) -> bool {
        loop {
            if scope == ancestor {
                return true;
            }

            let Some(parent) = schedule.scope_parent(scope) else {
                return false;
            };
            scope = parent;
        }
    }
}
