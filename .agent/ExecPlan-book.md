# Expand Boomerang User Book

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

Maintain this document in accordance with `.agent/PLANS.md` from the repository root.

## Purpose / Big Picture

Users should be able to learn Boomerang from the book without reading source code. After this work, a new user can follow the book to set up a minimal reactor, run it, understand the core concepts (reactors, reactions, actions, time/tags), and navigate advanced features like recording/replay. They can verify success by running example commands and seeing expected output in the terminal or by building the book with `mdbook build book` and navigating the rendered pages.

## Progress

- [x] (2026-01-22 14:09Z) Drafted initial ExecPlan for expanding the book.
- [x] (2026-01-22 14:12Z) Reviewed LF """A First Reactor""" and Writing Reactors outline for structure inspiration.
- [x] (2026-01-22 14:13Z) Defined the revised book information architecture and updated `book/src/SUMMARY.md`.
- [x] (2026-01-22 14:18Z) Wrote the Quickstart walkthrough with a runnable Hello World example.
- [x] (2026-01-22 14:18Z) Added the """Writing Reactors""" section modeled on LF""™s flow, adapted to Boomerang""™s Rust macros.
- [x] (2026-01-22 14:18Z) Added core concepts chapters (reactors, reactions, actions/ports, time/tags, determinism).
- [x] (2026-01-22 14:18Z) Added guides for composition, timers, state, and testing with `boomerang_util::runner`.
- [x] (2026-01-22 14:18Z) Expanded advanced topics: recording/replay and runtime execution model.
- [ ] Validate book build and fix broken links or code snippets.

## Surprises & Discoveries

- Observation: The LF """A First Reactor""" page emphasizes a minimal example, project structure, reactor block anatomy, and comments, plus a broader """Writing Reactors""" sequence in the sidebar.
  Evidence: LF """A First Reactor""" headings and sidebar topics (Inputs/Outputs, Parameters/State, Time/Timers, Composition, Reactions, Actions, etc.).

## Decision Log

- Decision: Structure the book as a progressive journey: Introduction, Quickstart, Writing Reactors, Concepts, Guides, Advanced, and Glossary.
  Rationale: Mirrors LF""™s learning path while preserving Boomerang""™s Rust-first approach.
  Date/Author: 2026-01-22 / Codex

- Decision: Add a Boomerang-specific """Writing Reactors""" section inspired by LF""™s sidebar topics, but only include chapters that map to current Boomerang capabilities, and explicitly note unsupported features.
  Rationale: Keeps the book familiar to LF users without promising unsupported behavior.
  Date/Author: 2026-01-22 / Codex

- Decision: Source runnable examples from existing tests and docs in `boomerang/src/lib.rs` and `boomerang/tests/hello_world.rs`.
  Rationale: Reusing tested code reduces drift between docs and actual behavior.
  Date/Author: 2026-01-22 / Codex

## Outcomes & Retrospective

Not started. This section will record outcomes and lessons once milestones complete.

## Context and Orientation

The Boomerang book is an mdBook project located at `book/` with configuration in `book/book.toml`. Content lives in `book/src/`. The current skeleton consists of `book/src/SUMMARY.md`, `book/src/introduction.md`, `book/src/quickstart.md`, and `book/src/glossary.md`, plus a partially drafted `book/src/replay.md`. The README for the workspace is in `README.md`. Example code for a simple reactor is in `boomerang/src/lib.rs` (doc example) and `boomerang/tests/hello_world.rs`. A larger example exists in `examples/snake/`.

Key terms that will appear in this plan are defined here for a novice:

A reactor is a component that encapsulates state, inputs, outputs, and reactions. A reaction is a function that runs when its triggering actions are present. An action is a logical event that can carry data; actions can be timers, logical actions, or physical actions. A tag is a pair of logical time and microstep that defines ordering for deterministic execution. Determinism means that given the same sequence of physical action inputs with their tags, the runtime produces the same internal states and outputs.

## Plan of Work

First, restructure the book""™s table of contents in `book/src/SUMMARY.md` to add a new """Writing Reactors""" section modeled after LF""™s flow. The LF """A First Reactor""" page emphasizes a minimal example, a project layout explanation, reactor block anatomy, and comments. Boomerang will mirror those as:

- A First Reactor (minimal runnable Rust example using `#[reactor]` and `reaction!`).
- Structure of a Boomerang Project (Cargo workspace layout and where reactors live).
- Reactor Anatomy (macro syntax and the Rust equivalents of inputs/outputs/actions).
- Comments and Style (Rust comment conventions only).

Then, add Boomerang equivalents for LF""™s """Writing Reactors""" sidebar topics where the feature is supported:

- Inputs and Outputs (ports and typed actions as used in `boomerang::prelude`).
- Parameters and State Variables (`#[reactor(state = ...)]` and state access).
- Time and Timers (`timer!`, delays, tags).
- Composing Reactors (child reactors and connections).
- Reactions and Reaction Declarations (the `reaction!` macro and triggers).
- Actions (logical vs physical actions and determinism boundaries).

For LF topics that are likely not yet supported (modal reactors, deadlines, distributed execution, etc.), include a short """Not yet supported in Boomerang""" note rather than a full chapter. Do not imply parity where it does not exist.

Next, update `book/src/quickstart.md` as a hands-on version of """A First Reactor,""" keeping a clear minimal example and a step-by-step run. Include the output expectations in a small code block to mirror LF""™s """run""" instructions, but for Boomerang""™s Rust execution.

Then add core concept chapters in `book/src/concepts/` that define reactors, reactions, actions/ports, timers, time/tags, and determinism. Each concept chapter should include a compact example and a """What to remember""" paragraph.

Then add guides in `book/src/guides/` for composition and hierarchy (drawing from `boomerang/tests/hierarchy.rs`), timers and delays (from `boomerang/tests/after.rs` and `boomerang/tests/action_delay.rs`), and stateful reactors. Provide a guide on testing and inspection using `boomerang_util::runner` and `tracing_subscriber` as shown in `boomerang/tests/hello_world.rs`.

Finally, expand `book/src/replay.md` to include a clear description of what recording and replay mean in Boomerang, define """physical action""" again in that context, and add a conceptual walkthrough of how the recorder is injected. If implementation details are missing, explicitly state the current limitations and tie them to repository paths.

## Concrete Steps

1. Update `book/src/SUMMARY.md` to add """Writing Reactors""" with a sub-TOC inspired by LF""™s sections (A First Reactor, Inputs/Outputs, Parameters/State, Time/Timers, Composition, Reactions, Actions). Add """Not yet supported""" notes as short sections instead of full chapters when needed.
2. Create the new markdown files under `book/src/writing-reactors/` and subdirectories, keeping file paths ASCII-only.
3. Populate `book/src/writing-reactors/a-first-reactor.md` with a minimal runnable example and a short """Structure of a Boomerang project""" section.
4. Populate each """Writing Reactors""" file with Boomerang-native macros and examples, referencing the existing tests for correctness.
5. Update `book/src/quickstart.md` to provide the runnable Hello World walkthrough, matching the minimal example.
6. Expand concepts and guides, then update `book/src/glossary.md`.
7. Run `mdbook build book` from the repository root and verify success.

Expected transcript for the build step:

    $ pwd
    /Users/johhug01/Source/boomerang
    $ mdbook build book
    2026-01-22 ... INFO - Building book...
    2026-01-22 ... INFO - Output written to book/book

## Validation and Acceptance

Acceptance is reached when a novice can:

1. Build the book with `mdbook build book` from the repo root without errors.
2. Follow """A First Reactor""" to create and run a Hello World reactor and see printed output in the terminal.
3. Read the Writing Reactors section and map LF-like concepts to Boomerang macros without ambiguity.
4. Navigate the rendered book and see all pages linked in the summary, including the expanded recording/replay page.
5. Run `cargo test` from the repo root and see tests pass; no tests should break due to documentation changes.

## Idempotence and Recovery

All steps are additive. Re-running `mdbook build book` is safe and will overwrite the generated `book/book` output. If a markdown page is missing or a link is broken, rerun the build after fixing `book/src/SUMMARY.md` and the file path. If code snippets drift, refresh them from `boomerang/src/lib.rs` and `boomerang/tests/hello_world.rs`.

## Artifacts and Notes

Example `book/src/SUMMARY.md` outline excerpt:

    - [Introduction](./introduction.md)
    - [Quickstart](./quickstart.md)
    - [Writing Reactors](./writing-reactors/README.md)
      - [A First Reactor](./writing-reactors/a-first-reactor.md)
      - [Inputs and Outputs](./writing-reactors/inputs-outputs.md)
      - [Parameters and State](./writing-reactors/parameters-state.md)
      - [Time and Timers](./writing-reactors/time-timers.md)
      - [Composing Reactors](./writing-reactors/composing.md)
      - [Reactions](./writing-reactors/reactions.md)
      - [Actions](./writing-reactors/actions.md)
    - [Concepts](./concepts/README.md)
    - [Guides](./guides/README.md)
    - [Advanced](./advanced/README.md)
      - [Recording and Replay](./replay.md)
    - [Glossary](./glossary.md)

## Interfaces and Dependencies

The book depends on mdBook. Use `mdbook` for building and serving the book, and `cargo` for validating code examples where needed. Use the public re-exports from `boomerang::prelude::*` in code snippets, and the helper `boomerang_util::runner::build_and_test_reactor` for runnable examples. Avoid introducing new dependencies in the book content itself.

Plan update note: Updated the ExecPlan to incorporate LF """A First Reactor""" structure and the Writing Reactors topic sequence as inspiration, while explicitly scoping unsupported topics.

Plan update note: Marked documentation milestones complete after creating the new book sections and updating existing pages.
