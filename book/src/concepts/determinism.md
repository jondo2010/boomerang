# Determinism

Determinism means that given the same sequence of physical action inputs with the same tags, Boomerang will produce the same internal state transitions and outputs.

Logical actions and timers are deterministic within the runtime. Physical actions are the boundary between the deterministic system and the outside world.

What to remember: to reproduce behavior, record physical action inputs (value and tag) and replay them.
