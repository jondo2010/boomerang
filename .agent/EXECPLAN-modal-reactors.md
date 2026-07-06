# Full Modal Reactors

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

Reference: `.agent/PLANS.md` in the repository root. This ExecPlan must be maintained in accordance with that file.

## Purpose / Big Picture

Implement full modal reactor semantics in Boomerang, comparable in intent to Lingua Franca modal reactors. After this work, a Boomerang reactor can declare mutually exclusive modes whose contained reactions, timers, logical actions, child reactors, and delayed connections execute only while their enclosing mode is active. A reaction can transition to another mode, using reset behavior by default or history behavior when requested. Reset discards pending local events and reinitializes modal timing behavior; history suspends and later resumes local time as if no time passed while the mode was inactive.

The user-visible result is a reactor that can model behavior such as "idle" and "active" phases without manually guarding every reaction and without timers in inactive modes continuing to consume logical time. This will be demonstrated by integration tests ported from LF modal model examples, especially cases where a pending action in a history mode resumes later while a pending action in a reset mode is discarded.

## Progress

- [x] (2026-07-06 18:26Z) Replaced the prior minimal reaction-gating ExecPlan with this full-semantics modal reactors plan.
- [ ] Agree on the front-end syntax and macro integration details before implementation.
- [ ] Implement static mode scopes in the builder and runtime graph.
- [ ] Implement typed transition effects in reaction declarations.
- [ ] Implement mode-local event queues and local-time scheduling.
- [ ] Implement reset/history transition application, including recursive reset of contained modal reactors.
- [ ] Implement modal startup, shutdown, and reset-trigger behavior.
- [ ] Add LF-style integration tests and performance benchmarks.

## Surprises & Discoveries

- Observation: The prior WIP proves that per-reaction gating and deferred transition application can pass simple mixed-reaction tests, but that design is not enough for full modal reactors.
  Evidence: The WIP only stores reaction mode filters and current mode state; it has no ownership of timers, logical actions, delayed connections, child reactors, queued local events, reset/history transition kinds, or startup/shutdown special cases.

- Observation: To make mode declarations ergonomic inside `#[reactor]`, the reactor macro must parse mode blocks itself rather than relying on an ordinary Rust macro statement alone.
  Evidence: Forward references such as a reaction inside `idle` transitioning to `active` require the macro to discover all mode names before emitting builder code for any mode body.

## Decision Log

- Decision: Implement full LF-style modal semantics, not the minimal reaction-filter feature.
  Rationale: The feature is only useful as "modal reactors" if it controls modal components and local time, not just whether a reaction body is skipped.
  Date/Author: 2026-07-06 / Codex, confirmed by user.

- Decision: Parse mode syntax directly as part of the `#[reactor]` macro input.
  Rationale: This gives users structural mode blocks, supports forward references to modes, and lets the macro generate typed mode handles before expanding mode bodies.
  Date/Author: 2026-07-06 / Codex.

- Decision: Reset is the default transition kind; `history(mode_name)` explicitly requests history behavior.
  Rationale: This matches the LF convention and keeps common transitions concise.
  Date/Author: 2026-07-06 / Codex, confirmed by user.

- Decision: Do not introduce automatic Rust state field reset in the first full implementation. Use explicit reset reactions over existing Rust state.
  Rationale: Boomerang state is arbitrary Rust data created from `#[state]` function parameters. Automatically resetting selected fields would require a second state-definition DSL or trait bounds that do not fit existing ergonomics. LF already treats explicit reset reactions as the correct escape hatch for state reinitialization.
  Date/Author: 2026-07-06 / Codex.

- Decision: Use typed mode transition effects instead of `Context::set_mode_name(&'static str)`.
  Rationale: String lookup is slow, typo-prone, and bypasses builder validation. A transition effect declared in a reaction signature lets the builder validate ownership and transition kind once.
  Date/Author: 2026-07-06 / Codex.

- Decision: Represent local time with per-scope local event queues plus an active-scope frontier, not by scanning all queued events on every transition.
  Rationale: Performance depends on inactive modes being cheap. Suspending a mode should not require updating every event in the system, and finding the next event should consider only active scopes.
  Date/Author: 2026-07-06 / Codex.

## Outcomes & Retrospective

Not started. The expected outcome is a complete modal runtime with tests that prove reset/history local-time behavior and benchmark data showing that non-modal models retain current performance characteristics.

## Context and Orientation

Boomerang is a Rust workspace. User-facing macros live in `boomerang_macros`, builder-side model construction lives in `boomerang_builder`, runtime scheduling lives in `boomerang_runtime`, and integration tests live in `boomerang/tests`.

The existing reactor syntax is a Rust function annotated with `#[reactor]`. The macro provides a `builder` variable in the function body, and helper macros such as `timer!` and `reaction!` expand into builder calls. The current WIP on the branch before this plan added a minimal mode key, reaction filters, and transition after a tag. That code should be treated as a spike: useful evidence, but not the target architecture.

Important current files:

- `boomerang_macros/src/reactor.rs` parses the `#[reactor]` function and emits builder code.
- `boomerang_macros/src/reaction.rs` parses `reaction!` syntax and emits `builder.add_reaction(...).with_trigger(...).with_effect(...).finish()?`.
- `boomerang_builder/src/env/mod.rs` owns the builder graph: reactors, ports, actions, reactions, connections, and validation.
- `boomerang_builder/src/env/build.rs` lowers builder graph data into runtime `Enclave`s, `Env`, and `ReactionGraph`.
- `boomerang_runtime/src/env/mod.rs` defines runtime graph keys and static dependency maps.
- `boomerang_runtime/src/sched.rs` owns the event queue and reaction scheduling loop.
- `boomerang_runtime/src/store.rs` owns runtime reactor state, action stores, port stores, reaction objects, and cached reaction borrow contexts.

Terms used in this plan:

A mode is a named mutually exclusive state of one reactor. Exactly one sibling mode in a modal group is active at a logical instant.

A mode scope is the static region of the model contained by a mode. Reactions, timers, logical actions, child reactors, and delayed connections can belong to a mode scope. Root scope means the component is outside any mode and always active.

Local time is logical time measured only while a mode scope is active. If a mode is inactive for one second of global logical time, timers and scheduled logical actions in that mode do not age by one second.

A reset transition enters a mode as if its local timing history were new. It discards pending local events in the target scope and recursively resets contained modal reactors to their initial modes. Reset is the default transition kind.

A history transition enters a mode preserving its local timing history. Queued local timers and actions resume with the same remaining delay they had when the mode was deactivated.

A transition effect is a typed value passed to a reaction because the reaction declared that it may set a mode. It is not a port or action value. Calling `active.set(ctx)` records a transition request in the current reaction context.

## Proposed Front-End Syntax

The preferred user syntax integrates modes directly into `#[reactor]`. The macro parses `initial mode` and `mode` blocks as part of the reactor function body, discovers all mode names first, emits mode handles, and then emits scoped builder code for each mode body. This permits forward references between sibling modes.

Example:

    #[reactor]
    fn Controller(
        #[state] model: ControllerState,
        #[input] cmd: Command,
        #[output] status: Status,
    ) -> impl Reactor {
        timer! { heartbeat(0 sec, 1 sec) }

        reaction! {
            (heartbeat) {
                state.model.global_ticks += 1;
            }
        }

        initial mode idle {
            timer! { poll(0 sec, 100 msec) }

            reaction! {
                (startup) {
                    state.model.idle_started = true;
                }
            }

            reaction! {
                (reset) {
                    state.model.reset_idle();
                }
            }

            reaction! {
                (cmd) -> active, status {
                    if cmd.as_ref() == Some(&Command::Start) {
                        active.set(ctx);
                    }
                    *status = Some(Status::Idle);
                }
            }
        }

        mode active {
            timer! { tick(0 sec, 10 msec) }
            let work = builder.add_logical_action::<Work>("work", Some(Duration::milliseconds(500)))?;

            reaction! {
                (tick) -> work {
                    ctx.schedule_action(&mut work, Work::new(), None);
                }
            }

            reaction! {
                (cmd) -> history(idle), status {
                    if cmd.as_ref() == Some(&Command::Pause) {
                        idle.set(ctx);
                    }
                    *status = Some(Status::Active);
                }
            }
        }
    }

In the example, `heartbeat` and its reaction are outside modes and always active. `poll`, `tick`, `work`, and the mode-local reactions are scoped to their enclosing mode. `-> active` declares a reset transition effect because reset is the default. `-> history(idle)` declares a history transition effect. The closure argument named `active` or `idle` is a typed transition handle, not a string. Calling `.set(ctx)` schedules the mode change; merely declaring the effect does not transition.

Mode syntax is intentionally restricted:

Ports remain reactor-level declarations. A mode may use reactor-level ports in reactions and connections, but may not declare new input or output ports because ports are the stable interface of a reactor.

Nested `mode` blocks are not allowed directly inside another mode. Hierarchical modal behavior is expressed by instantiating a child reactor that itself has modes inside a parent mode. This matches LF's practical composition model and keeps the builder graph simple.

An unqualified mode effect such as `-> active` means reset. The explicit spelling `-> reset(active)` is accepted as an alias. The spelling `-> history(active)` requests history.

The `reaction!` macro gains a `reset` trigger keyword. A reset-triggered reaction is valid only inside a mode scope and runs when that mode is entered by reset behavior. A reset reaction is not a transition effect; it is a reaction trigger.

The lower-level builder syntax remains available for generated code and tests:

    let idle = builder.add_mode("idle", ModeKind::Initial)?;
    let active = builder.add_mode("active", ModeKind::Normal)?;
    builder.in_mode(idle, |builder| {
        builder.add_reaction(Some("enter_idle"))
            .with_reset_trigger()
            .with_reaction_fn(|ctx, state, ()| {
                state.model.reset_idle();
            })
            .finish()?;
        Ok(())
    })?;

This lower-level API is not the primary documentation surface, but it provides a stable target for macro expansion.

## Runtime Semantics

At startup, each modal reactor activates its initial mode. Components outside modes are active immediately. Startup reactions outside modes run at program startup as they do today. Startup reactions inside a mode run at most once, when that mode scope is first activated. Shutdown reactions inside modes run when the reactor shuts down if their enclosing mode scope has been activated at least once, even if the mode is inactive at shutdown.

When a reaction sets a mode transition at tag `(t, m)`, the current mode remains active for the rest of that tag. The target mode can first produce reactions at a future tag. If a reset activation produces an immediate reset reaction or a zero-offset timer, schedule it at `(t, m + 1)`. If no immediate local event exists, the target mode waits for its next local event or external trigger.

Multiple reactions may request transitions in the same tag. The scheduler uses existing deterministic reaction order; the last executed transition request for a modal reactor wins. If a reaction declares two transition effects and sets both, the last `.set(ctx)` call inside that reaction wins for that reactor.

For reset transitions, the runtime discards pending local events owned by the target scope and all recursively contained scopes, resets contained modal reactors to their initial modes, clears activation history for contained startup where appropriate, schedules reset-triggered reactions, and schedules initial timer offsets for the entered scope. Rust state is reset only by user-written reset reactions.

For history transitions, the runtime preserves queued local events. While the scope was inactive, its local clock did not advance, so each queued local event keeps the same remaining local delay. On reactivation, the runtime maps the next local event to global time based on the reactivation tag.

Mode-local logical actions, timers, and delayed connections are scheduled in local time. Root-scope actions, timers, and connections keep current global behavior.

External physical actions require careful treatment because physical events happen in wall-clock time, not logical local time. For the first full implementation, physical actions declared in a mode should be accepted but their incoming events should be delivered only if their enclosing mode is active at the event tag; they are not suspended and replayed by history. This behavior must be documented and tested. If this proves surprising during implementation, record the discovery and consider rejecting mode-local physical actions until a better semantic model is designed.

## Performance Design

The performance target is that non-modal programs pay only small constant overhead, and modal programs do not scan inactive modes at every tag.

Static graph data should be computed once in `boomerang_builder/src/env/build.rs` and stored in `boomerang_runtime/src/env/mod.rs`. Every reaction, action, timer, port connection, and reactor instance gets a compact `ScopeKey`. Root scope is represented by a sentinel or `ScopeKey::from(0)`. Each scope stores its parent scope, owning reactor, optional mode key, initial child mode if it is a modal group, and precomputed descendants for reset. For active checks, store a precomputed ancestor chain or compact bitset. If the number of scopes is small enough to fit in a machine word, use an inline mask; otherwise store ranges over a flattened scope tree.

The scheduler should avoid per-tag allocation. Add reusable buffers to `Scheduler` for transition requests, reset scopes, and reaction filtering. Do not allocate a new vector while iterating levels. Existing WIP added reusable buffers; keep that idea but move it behind full scope semantics.

Use per-scope local event queues. A local event stores a local tag, an event payload, the owning scope, and a generation number. Each active scope has at most one frontier entry in a global frontier heap: the global tag corresponding to the head of that scope's local queue. When a local queue changes or a scope is activated, push a new frontier entry with that scope's queue epoch. Stale frontier entries are skipped by comparing epoch and active generation. This makes finding the next global event proportional to the number of active scopes with pending events, not all inactive events.

Reset should invalidate old events by incrementing a scope generation and clearing that scope's local queues. If clearing a large queue is too expensive, replace the queue with an empty queue and let old heap entries expire by generation. The reset cost should be proportional to the events owned by the reset subtree, not the entire model.

History should not rewrite queued local tags. On deactivation, record the local time at suspension. On activation, record the global tag where the scope resumed and map local queue heads to global tags lazily as `activation_global_tag + (event_local_tag - activation_local_tag)`. This avoids touching every queued event on history transition.

Reaction gating remains necessary because root-scope ports can trigger reactions inside inactive modes. The check must be a cheap scope-active check at execution time and when extending downstream levels from set ports. It must not perform name lookup, heap allocation, or scan ancestor chains in the hot loop.

Benchmark requirements:

- `cargo bench -p boomerang --bench ping_pong` should show no meaningful regression for non-modal models.
- A new modal benchmark should include many inactive modes each with timers/actions queued, proving inactive modes do not add per-tag cost.
- If `BOOMERANG_PROFILE=1 cargo bench -p boomerang --bench modal_modes` is run, the flamegraph should show the scheduler spending time on active event queues and reaction execution, not scanning inactive scopes.

## Plan of Work

First, remove or quarantine the minimal WIP API from the previous spike. The old `@modes(...)`, `@transition(...)`, and `Context::set_mode_name` design should not become the long-term API. If retaining compatibility temporarily helps development, put it behind private tests or feature-gated spike code and delete it before acceptance.

Next, extend `boomerang_builder` with static scopes. Add builder keys for modes and scopes, store each component's scope, and add validation rules: each modal reactor has exactly one initial mode; mode names are unique per reactor; mode effects may only target sibling modes of the current modal reactor; reset triggers are only valid inside modes; mode-local ports are rejected; direct nested mode blocks are rejected.

Then extend `boomerang_macros/src/reactor.rs` so the `#[reactor]` parser recognizes `initial mode name { ... }` and `mode name { ... }` blocks. The parser should collect mode declarations before emitting code, then emit builder calls to create all sibling mode handles, then emit each mode body inside a scoped builder. Extend `boomerang_macros/src/reaction.rs` to parse `reset` as a trigger and `reset(mode)` / `history(mode)` / bare `mode` as mode transition effects.

After the builder and macro layers compile, add runtime static graph support in `boomerang_runtime/src/env/mod.rs`. Define `ScopeKey`, `ModeKey`, `TransitionKind`, `ModeTransitionEffect`, and maps from reactions/actions/timers/connections to scopes. Keep public exports minimal; most keys should remain runtime/builder implementation details unless user code must name them.

Next, refactor scheduling. `boomerang_runtime/src/sched.rs` currently uses a global event queue. Introduce a root global queue plus per-scope local queues or one generalized event manager that treats root as always-active local time. All public scheduler behavior must remain deterministic. Keep existing tests passing after each refactoring step by making root-scope behavior equivalent to today's behavior before enabling mode-local behavior.

Then implement transition application after each tag. Collect transition requests from triggered reactions, resolve last-wins per modal reactor, and apply reset/history activation. Activation must schedule any immediate local events at the next microstep, not in the same tag.

Finally, implement startup/shutdown/reset modal triggers and recursive reset behavior. Add tests first or alongside implementation so failures describe the missing semantics precisely.

## Concrete Steps

From repository root `/Users/johhug01/Source/boomerang`, start by confirming the branch and baseline:

    git branch --show-current

Expected output:

    modal-reactors2

Run the current tests before implementation begins:

    cargo test

Expected output is all current tests passing. If this fails before modal work begins, fix or record the pre-existing failure in `Surprises & Discoveries`.

Add syntax parser tests in `boomerang_macros/src/reactor.rs` and `boomerang_macros/src/reaction.rs`. Include tests for:

- two sibling mode blocks, one initial;
- forward transition reference from the first mode body to the second mode;
- `-> active` and `-> reset(active)` both parse as reset;
- `-> history(idle)` parses as history;
- `(reset)` parses as a reset trigger;
- direct nested `mode` blocks fail with a clear diagnostic.

Add builder validation tests in `boomerang_builder/src/tests.rs` for duplicate mode names, missing initial mode, multiple initial modes, transition to mode in another reactor, reset trigger outside mode, and mode-local port declarations.

Add runtime tests under `boomerang/tests/`:

- `modal_basic.rs`: a reset transition toggles modes and only the active mode's reaction runs.
- `modal_mixed_reactions.rs`: top-level reactions and mode reactions execute in deterministic order, and a newly active mode does not run in the same tag.
- `modal_history_local_time.rs`: a mode schedules a logical action, exits before the action fires, re-enters by history, and the action fires after the remaining local delay.
- `modal_reset_discards_events.rs`: a mode schedules a logical action, exits before it fires, re-enters by reset, and the old action never fires.
- `modal_reset_recursive.rs`: resetting a parent mode resets contained child modal reactors to their initial modes.
- `modal_startup_shutdown.rs`: startup inside a mode runs once on first activation, and shutdown inside a previously activated mode runs at program shutdown even if the mode is inactive.

After each milestone, run the narrowest useful test:

    cargo test -p boomerang_macros modal
    cargo test -p boomerang_builder modal
    cargo test -p boomerang_runtime modal
    cargo test -p boomerang modal

Before completion, run:

    cargo fmt --check
    cargo test
    cargo bench -p boomerang --bench ping_pong
    cargo bench -p boomerang --bench modal_modes

## Validation and Acceptance

The feature is accepted only when a user can write one `#[reactor]` function with `initial mode` and `mode` blocks, declare timers/actions/reactions inside those blocks, transition with reset and history effects, and observe correct local-time behavior.

Behavioral acceptance:

When a reaction inside `idle` sets `active` at tag `(t, m)`, no reaction inside `active` executes at `(t, m)`. If `active` has a zero-offset timer or reset reaction, its earliest reaction occurs at `(t, m + 1)`.

When a pending local action is scheduled in a mode and the mode is exited, that action does not fire while the mode is inactive. If the mode is re-entered by history, the action fires after the remaining local delay. If the mode is re-entered by reset, the action is discarded.

When a parent mode is reset, contained modal reactors return to their initial modes and their pending local events are discarded.

When a startup reaction is inside a mode, it runs at most once on first activation. When a shutdown reaction is inside a mode, it runs at program shutdown if its mode has ever been active.

Performance acceptance:

Non-modal `ping_pong` benchmark results should stay within normal noise of the baseline. If the median changes significantly, investigate before merging. The modal benchmark should demonstrate that adding many inactive modes does not increase per-tag scheduler cost linearly with all inactive events.

## Idempotence and Recovery

The implementation should be done in small commits. Each milestone should leave the workspace compiling or should explicitly mark a temporary expected failure in `Progress`. If a parser or builder refactor fails, it is safe to revert only that milestone commit and keep this ExecPlan. Do not use destructive git commands unless explicitly requested by the user.

Generated benchmark artifacts under `target/` should not be committed. Runtime or tool logs under `logs/` should not be committed unless a future plan explicitly says otherwise.

If the local-time scheduler refactor becomes too large, split it behind a new internal `EventManager` type while preserving the old root-scope behavior. Keep all existing non-modal tests passing before adding modal local-time behavior.

## Artifacts and Notes

Prior WIP checkpoint:

    2026-07-06: Current branch `modal-reactors` was committed as `964ef3b feat: add modal reactors spike` before creating `modal-reactors2`.

The old WIP is useful for these ideas:

- store current mode state per reactor;
- defer transition application until after a tag;
- use reusable scheduler buffers;
- validate mode ownership in the builder.

The old WIP should not be copied directly for these API decisions:

- `@modes(...)` as the main syntax;
- `@transition(...)` without reset/history kind;
- `Context::set_mode_name(&'static str)`;
- treating modes as reaction filters only.

## Interfaces and Dependencies

No new external crates are expected. Use existing `slotmap` for builder keys and `boomerang_tinymap` for runtime maps. If a compact bitset is needed, first evaluate whether a small inline representation is enough before adding a dependency.

In `boomerang_builder/src/reactor.rs`, define builder-side keys and data:

    slotmap::new_key_type! { pub struct BuilderModeKey; }
    slotmap::new_key_type! { pub struct BuilderScopeKey; }

    pub enum ModeKind {
        Initial,
        Normal,
    }

    pub enum TransitionKind {
        Reset,
        History,
    }

In `boomerang_builder/src/env/mod.rs`, add mode and scope builders:

    pub struct ScopeBuilder {
        pub parent: Option<BuilderScopeKey>,
        pub reactor_key: BuilderReactorKey,
        pub mode_key: Option<BuilderModeKey>,
    }

    pub struct ModeBuilder {
        pub name: String,
        pub reactor_key: BuilderReactorKey,
        pub scope_key: BuilderScopeKey,
        pub kind: ModeKind,
    }

ReactorBuilderState should gain scoped builder support:

    pub fn add_mode(&mut self, name: &str, kind: ModeKind) -> Result<BuilderModeKey, BuilderError>;
    pub fn in_mode<R>(&mut self, mode: BuilderModeKey, f: impl FnOnce(&mut Self) -> Result<R, BuilderError>) -> Result<R, BuilderError>;

The exact lifetime shape may differ; the important interface is that macro-generated code can enter a mode scope and existing `timer!`, `reaction!`, child reactor, action, and connection builder calls record that scope.

In `boomerang_builder/src/reaction.rs`, add mode transition effects:

    pub struct BuilderModeEffect {
        pub target: BuilderModeKey,
        pub transition: TransitionKind,
    }

    pub trait PartialReactionBuilderField {
        // Existing behavior remains for ports/actions.
    }

    impl PartialReactionBuilderField for BuilderModeEffect { ... }

Add `with_reset_trigger()` for reset reactions. Reset triggers are not action keys; they are mode-scope lifecycle triggers lowered into runtime reset reactions.

In `boomerang_runtime/src/env/mod.rs`, define runtime keys and graph fields:

    tinymap::key_type! { pub ScopeKey }
    tinymap::key_type! { pub ModeKey }

    pub enum TransitionKind {
        Reset,
        History,
    }

    pub struct ScopeInfo {
        pub parent: Option<ScopeKey>,
        pub reactor: ReactorKey,
        pub mode: Option<ModeKey>,
        pub descendants: Vec<ScopeKey>,
    }

    pub struct ModeInfo {
        pub reactor: ReactorKey,
        pub scope: ScopeKey,
        pub name: String,
        pub initial: bool,
    }

ReactionGraph should include scope ownership maps:

    pub scopes: TinyMap<ScopeKey, ScopeInfo>;
    pub modes: TinyMap<ModeKey, ModeInfo>;
    pub reaction_scopes: TinySecondaryMap<ReactionKey, ScopeKey>;
    pub action_scopes: TinySecondaryMap<ActionKey, ScopeKey>;
    pub port_connection_scopes: Vec<ConnectionScopeInfo>;
    pub reset_reactions: TinySecondaryMap<ScopeKey, Vec<LevelReactionKey>>;
    pub startup_reactions: TinySecondaryMap<ScopeKey, Vec<LevelReactionKey>>;
    pub shutdown_reactions_by_scope: TinySecondaryMap<ScopeKey, Vec<LevelReactionKey>>;

In `boomerang_runtime/src/context.rs`, replace string mode scheduling with typed transition recording:

    pub(crate) struct TriggerRes {
        pub scheduled_actions: Vec<(ActionKey, Tag)>,
        pub scheduled_shutdown: Option<Tag>,
        pub scheduled_mode: Option<ModeTransitionRequest>,
    }

    pub struct ModeEffectRef {
        target: ModeKey,
        transition: TransitionKind,
    }

    impl ModeEffectRef {
        pub fn set(&self, ctx: &mut Context) { ... }
    }

In `boomerang_runtime/src/sched.rs`, introduce an event manager:

    struct EventManager {
        root_queue: BinaryHeap<ScheduledEvent>,
        scope_queues: TinySecondaryMap<ScopeKey, LocalEventQueue>,
        active_frontier: BinaryHeap<ScopeFrontierEntry>,
    }

The exact names can change, but the design must preserve root-scope behavior and make inactive local queues dormant without scanning them on each global tag.

Change log: 2026-07-06 / Codex: replaced minimal modal reactors plan with full-semantics plan after user confirmed full LF-like semantics, reset default, and branch serialization workflow.
