# Recording and Replay

The requirement is the ability to record necessary inputs and states of a running system, serialize it into one or more files on disk, and then be able to deterministically replay that recording back into the runtime "offline", at potentially faster-than-realtime speed.

The determinism in Boomerang here means that a system being fed recorded data should repeatably achieve the exact same state and provide the same bitwise-exact outputs as if it was running with real inputs.

In Boomerang, `PhysicalAction`s are the boundary between the deterministic runtime and the non-deterministic outside world. Any external sensor or data inputs to the system *must* enter through a `PhysicalAction`. This means to achieve a perfect replay capability, it is only necessary to record the time (`Tag`) and values of the `PhysicalActions` of the system. This is what the recording and replay infrastructure is centered around.

# Design

Actions in Boomerang are (logically) local to the Reactor that contains them, and are not accesible to other Reactors.

The Recorder works by injecting an additional Reaction into the containing Reactor for each `PhysicalAction` that should be recorded.

# Serialization Data Model

