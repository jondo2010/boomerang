# Glossary

## History Transition

A mode transition that preserves the target mode's local timing state. Pending mode-local logical actions, timers, and delayed connections resume with the same remaining local delay they had when the mode became inactive.

## Local Time

Logical time measured only while a mode scope is active. Mode-local timers, logical actions, and delayed connections use local time.

## Mode

A named state of a reactor. Exactly one sibling mode is active at a logical instant.

## Mode Scope

The static region of a reactor contained by a mode. Reactions, timers, logical actions, child reactors, and delayed connections declared inside the mode belong to that scope.

## Reset Transition

The default mode transition. It enters the target mode with fresh local timing state, discards pending mode-local events in the reset scope, and returns contained modal child reactors to their initial modes.
