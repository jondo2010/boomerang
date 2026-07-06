# Stabilize PlantUML Reactor Graph Output

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This plan must be maintained in accordance with `.agent/PLANS.md` from the repository root.

## Purpose / Big Picture

Boomerang can already generate PlantUML diagrams from Reactor hierarchies, but the current output can be incorrect or incomplete. After this change, a user can generate a `.puml` file that reliably includes every Reactor, uses stable identifiers without collisions, and safely renders names containing special characters. They will see complete, deterministic diagrams for any Reactor hierarchy, including multiple top-level reactors and banked ports.

## Progress

- [x] (2026-01-22T11:54Z) Reviewed current PlantUML generator (`boomerang_builder/src/plantuml.rs`) and call site (`boomerang_util/src/runner.rs`).
- [x] (2026-01-22T11:57Z) Implemented stable, collision-free PlantUML node IDs with a shared helper.
- [x] (2026-01-22T11:57Z) Ensured all root reactors are traversed and emitted, not just the first.
- [x] (2026-01-22T11:57Z) Added PlantUML-safe label escaping and applied it to ports, reactions, actions, and tooltips.
- [x] (2026-01-22T11:57Z) Improved banked connection rendering in `build_port_bindings`.
- [x] (2026-01-22T11:57Z) Added unit tests that prove the new behavior and prevent regressions.
- [x] (2026-01-22T11:58Z) Ran `cargo test -p boomerang_builder plantuml` and validated the new PlantUML tests.

## Surprises & Discoveries

Document unexpected behaviors, bugs, optimizations, or insights discovered during implementation. Provide concise evidence.

- Observation: PlantUML graph traversal starts DFS from only the first toposorted reactor, so additional root reactors are never emitted.
  Evidence: `create_plantuml_graph` in `boomerang_builder/src/plantuml.rs` uses `ordered_reactors.first()` and a single DFS.
- Observation: Node IDs are created by `key.data().as_ffi() % len`, which can collide when slotmap keys are sparse or reused.
  Evidence: `ElemId` implementations in `boomerang_builder/src/plantuml.rs` use modulo by `*_builders.len()`.

## Decision Log

- Decision: Use `Key::data().as_ffi()` (without modulo) as the base for stable IDs, prefixed per node type.
  Rationale: Slotmap keys are already unique within a map; modulo truncation is the source of collisions.
  Date/Author: 2026-01-22 / Codex
- Decision: Traverse all root reactors in a deterministic order rather than only the first root.
  Rationale: Multiple top-level reactors are a valid environment and should appear in the diagram.
  Date/Author: 2026-01-22 / Codex
- Decision: Introduce a small PlantUML escaping helper and apply it to all quoted labels.
  Rationale: User-defined names can contain "\"", `]`, or newlines, which break PlantUML syntax.
  Date/Author: 2026-01-22 / Codex

## Outcomes & Retrospective

No implementation yet. This section will be updated after changes land and tests validate the behavior.

## Context and Orientation

PlantUML output is generated in `boomerang_builder/src/plantuml.rs` by `EnvBuilder::create_plantuml_graph`. The output is written to disk by `boomerang_util/src/runner.rs` when the CLI flag `--reaction-graph` is used. Reactor, port, reaction, and action builders are stored in slotmaps, and keys are `slotmap::Key` types. Banked reactors and ports have `BankInfo` and are grouped via `EnvBuilder::ports_debug_grouped` and `EnvBuilder::reactors_debug_grouped`.

The current generator relies on modulo arithmetic when creating node IDs (e.g. `key.data().as_ffi() % len`), which is unsafe for uniqueness because slotmaps can have gaps or reused indices. The generator currently runs DFS from only the first node in a toposort, so any additional root reactors are not emitted. Labels are inserted directly into PlantUML strings without escaping.

## Plan of Work

First, introduce a small helper in `boomerang_builder/src/plantuml.rs` to build node IDs for each element type. The helper must take a slotmap key and return a stable identifier string without modulo, e.g. `rtr<hex>`, `p<hex>`, `r<hex>`, `a<hex>`. Apply this helper in all locations that currently compute IDs directly.

Second, update traversal in `EnvBuilder::create_plantuml_graph` to cover all roots. Collect the toposorted list and iterate through it, launching DFS for each node that has no parent and is not yet visited. Maintain a visited set so nodes are emitted once.

Third, add a PlantUML string escaping function in `boomerang_builder/src/plantuml.rs` that replaces backslashes, quotes, and closing bracket sequences that interfere with `[[...]]` links. Apply it to reactor names, port names, reaction names, action names, and tooltips.

Fourth, update `build_port_bindings` to group banked ports by FQN. For each connection, map source and target keys to their grouped bank representative (similar to `ports_debug_grouped`) and emit a single edge for the banked connection, using `EnvBuilder::BANK_EDGE` thickness for banked connections. This should reduce visual noise and align with the intended banked representation.

Finally, add unit tests in `boomerang_builder/src/tests.rs` or a new `#[cfg(test)]` module in `boomerang_builder/src/plantuml.rs` that:
- Build an environment with two top-level reactors and assert both names appear in the `.puml` output.
- Create ports or reactors with names containing quotes/brackets and assert the output is valid escaped form.
- Create a banked port and assert that only one edge is emitted with the bank thickness marker.
- Confirm that node IDs are unique and do not reuse modulo values.

## Concrete Steps

From repository root (`/Users/johhug01/Source/boomerang`), edit files and run tests in this order.

1. Update `boomerang_builder/src/plantuml.rs`.
   - Add `fn puml_id_for_reactor(...)`, `fn puml_id_for_port(...)`, `fn puml_id_for_reaction(...)`, `fn puml_id_for_action(...)`.
   - Add `fn escape_puml_label(input: &str) -> String`.
   - Replace all inline ID computations with the helper functions.
   - Update traversal to cover all roots with a visited set.
   - Update `build_port_bindings` to group banked ports.

2. Add tests.
   - If placing in `boomerang_builder/src/plantuml.rs`, add a `#[cfg(test)] mod tests` section.
   - Build minimal `EnvBuilder` instances and assert key output substrings.

3. Run targeted tests.
   - In the repository root, run `cargo test -p boomerang_builder plantuml`.

Expected snippet in output for a multi-root environment should include both reactor names. Example (illustrative).

    component rtr1 as "RootA"{
    component rtr2 as "RootB"{

## Validation and Acceptance

Acceptance is achieved when:

- Running `cargo test -p boomerang_builder plantuml` passes.
- `EnvBuilder::create_plantuml_graph` output includes all top-level reactors in a multi-root environment.
- Names containing special characters are escaped and do not break the PlantUML file.
- Banked port connections appear as a single edge with the bank thickness marker rather than multiple duplicate edges.
- Node identifiers are unique and stable for a single generator run (no collisions from modulo).

## Idempotence and Recovery

These changes are safe to repeat. If a change introduces invalid PlantUML output, revert the helper function and re-run tests to confirm baseline behavior. If tests fail due to ordering differences, adjust tests to check for the presence of substrings rather than exact ordering.

## Artifacts and Notes

Short sample of expected output once fixed (illustrative only).

    component rtr1 as "RootA"{
      portin "in[0..1]" <<bank>> as p1
    }
    component rtr2 as "RootB"{
    }
    p1 .[thickness=2]-> r10 : trig

## Interfaces and Dependencies

- `boomerang_builder/src/plantuml.rs`
  - Define:
    - `fn escape_puml_label(input: &str) -> String`
    - `fn puml_id_for_reactor(key: BuilderReactorKey) -> String`
    - `fn puml_id_for_port(key: BuilderPortKey) -> String`
    - `fn puml_id_for_reaction(key: BuilderReactionKey) -> String`
    - `fn puml_id_for_action(key: BuilderActionKey) -> String`
  - Update:
    - `EnvBuilder::create_plantuml_graph`
    - `EnvBuilder::build_port_bindings`
    - `EnvBuilder::puml_write_ports`
    - `EnvBuilder::puml_write_reaction_nodes`
    - `EnvBuilder::puml_write_action_nodes`
    - `EnvBuilder::puml_write_reaction_edges`
    - `EnvBuilder::puml_write_action_edges`

- `boomerang_builder/src/tests.rs` or `boomerang_builder/src/plantuml.rs` test module
  - Add tests that build small `EnvBuilder` graphs and check output substrings.

Revision Note: (2026-01-22T11:57Z) Updated Progress to reflect implemented PlantUML changes and added tests; validation remains pending.
Revision Note: (2026-01-22T11:58Z) Updated Progress after running the targeted PlantUML tests.
