# Recording and Replay

Recording and replay lets you capture external inputs and reproduce a run deterministically. In Boomerang, the boundary between deterministic execution and the outside world is the physical action.

## What gets recorded

To replay a run, Boomerang only needs the values and tags of physical actions. If those inputs arrive with the same tags in the same order, the runtime produces the same internal state and outputs.

## How it works

Actions in Boomerang are local to the reactor that owns them. The recorder injects an additional reaction into each reactor that owns a physical action. That reaction captures the action value and the tag at which it arrives.

## Design constraints

- Only physical actions need to be recorded.
- Logical actions, timers, and internal reactions remain deterministic and do not require recording.
- Replays can run faster than wall clock time when using fast-forward configuration.

## Current status

The recording and replay infrastructure is still evolving. When implementing or extending it, use the existing runtime modules as the source of truth. If a feature is missing, document the gap and add a minimal test that fails before the change and passes after.
