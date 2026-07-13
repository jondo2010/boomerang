# Modal Reactors

Modal reactors let one reactor contain several named modes where only one sibling mode is active at a logical instant. A mode is useful when a reactor has distinct phases, such as idle and active, and each phase owns different reactions, timers, actions, child reactors, or delayed connections.

The main user-visible effect is that work declared inside an inactive mode does not run. Mode-local logical time is suspended while a mode is inactive, so timers, logical actions, and delayed connections inside that mode do not age until the mode becomes active again.

## Basic Syntax

Declare modes inside a `#[reactor]` function with `mode!` blocks. Exactly one sibling mode is marked `initial`.

```rust,ignore
use boomerang::prelude::*;

#[reactor]
fn Controller(
    #[state] ticks: u32,
    #[input] cmd: Command,
    #[output] status: Status,
) -> impl Reactor {
    mode! { initial idle {
        reaction! {
            (startup) {
                state.ticks = 0;
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
    } }

    mode! { active {
        let work = ctx.add_logical_action::<()>("work", Some(Duration::milliseconds(50)))?;
        let tick = ctx.add_timer(
            "tick",
            TimerSpec::default().with_period(Duration::milliseconds(10)),
        )?;

        reaction! {
            (tick) -> work {
                state.ticks += 1;
                ctx.schedule_action(&mut work, (), None);
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
    } }
}
```

The names listed after `->` in a reaction are effects. A mode effect is a typed transition handle, not a string. Calling `active.set(ctx)` requests the transition. Declaring `-> active` by itself does not change the mode.

## Reset And History

The default transition kind is reset. These spellings are equivalent:

```rust,ignore
reaction! {
    (cmd) -> active {
        active.set(ctx);
    }
}

reaction! {
    (cmd) -> reset(active) {
        active.set(ctx);
    }
}
```

A reset transition enters the target mode with fresh local timing state. Pending mode-local logical actions, timers, and delayed connection deliveries in the target mode are discarded and restarted according to their declarations. Child reactors inside the reset mode return to their own initial modes. Rust state is not reset automatically; use a reset reaction when state must be restored.

A history transition preserves the target mode's local timing state:

```rust,ignore
reaction! {
    (cmd) -> history(active) {
        active.set(ctx);
    }
}
```

If a mode-local action had 2 ms remaining when the mode became inactive, it still has 2 ms
remaining when the mode is re-entered by history. History also preserves logical microstep ordering
for pending work at the activation tag, so multiple zero-delay local actions resume in the same order
they would have had if the mode had stayed active.

## Lifecycle Reactions

Modes can contain startup, reset, and shutdown reactions.

```rust,ignore
mode! { active {
    reaction! {
        (startup) {
            state.entered_active = true;
        }
    }

    reaction! {
        (reset) {
            state.reset_for_active();
        }
    }

    reaction! {
        (shutdown) {
            state.active_was_seen = true;
        }
    }
} }
```

A mode-local `(startup)` reaction runs once, when that mode scope first becomes active. If the mode is initial, startup runs at program startup. If the mode is reached later by a transition, startup runs at the next microstep after the transition.

A `(reset)` reaction runs when its mode is entered by reset. Initial modes do not run reset reactions merely because the program started.

A mode-local `(shutdown)` reaction runs at program shutdown if its mode scope has been active at least once, even if it is inactive when shutdown happens. Shutdown reactions in unreachable modes do not run.

## Local-Time Components

The following declarations are mode-local when written inside a `mode!` block:

- reactions;
- timers;
- logical actions;
- child reactors;
- delayed connections created by connecting ports with an `after` delay.

Root-scope components, declared outside all modes, keep the usual global behavior and are always active.

Mode-local ports are not allowed. Ports are the stable interface of a reactor, so declare input and output ports at the reactor level and use them from mode-local reactions as needed.

Directly nested `mode!` blocks are not allowed. To model nested modal behavior, instantiate a child reactor inside a mode and give that child reactor its own modes.

Dependency cycles are checked against these static scopes. Reactions in sibling modes of the same
reactor are mutually exclusive, and this remains true for child reactors declared inside those
sibling modes. A dependency path that would be cyclic only by combining reactions from mutually
exclusive modes is allowed because those reactions cannot run together at one logical instant.

## Transition Timing

When a reaction requests a transition at tag `(t, m)`, the current mode remains active for the rest of that tag. Work in the target mode can first run at a later tag. Immediate mode-local work, such as a reset reaction, startup reaction, or zero-offset timer, runs at `(t, m + 1)`.

If multiple reactions request transitions for the same reactor at the same tag, deterministic reaction order decides the result and the last executed request wins. If one reaction sets two transition handles for the same reactor, the last `.set(ctx)` call in that reaction wins.

## Physical Actions

Physical actions are accepted inside modes, but they are not suspended as local-time events. Physical actions are scheduled from wall-clock time. If the physical event is processed while its mode is active, its reactions can run. If the mode is inactive at that event tag, those reactions do not run, and history re-entry does not replay the physical event.

Use logical actions, timers, or delayed connections when a mode-local event should pause while inactive and resume on history entry.
