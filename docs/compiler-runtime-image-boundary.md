# Compiler-to-Runtime Image Boundary Simplification

## Context

Deployment compilation has two distinct representations:

1. semantic compiler models, including `ApplicationTopology`,
   `ResolvedDeployment`, and analysis results; and
2. runtime images, including `OwnedCompiledDeployment`,
   `OwnedEnclaveImage`, `OwnedRtiImage`, and their borrowed schema views.

The semantic models describe identity, ownership, placement, and relationships
using stable IDs. Runtime images replace those identities with compact typed
keys and dense tables. Lowering is the one-way representation boundary between
them.

The current issue #225 branch improves typed runtime keys, but parts of the
implementation promote runtime indexing into compiler-side abstractions too
early. `EnclaveDomains` is the clearest example: it combines stable compiler
record selections with runtime index registries and spans. Separately,
generated launchers construct and validate a `FederateSliceView` only to
extract its `FederateIndex` before complete-deployment validation runs again.

This work is pre-release, so compatibility for the affected APIs is not
required.

## Architectural Invariants

- Semantic compiler models retain stable IDs. They do not expose runtime keys,
  `TinyMap`, or runtime table spans.
- Runtime image tables use distinct typed keys end to end. Complete tables use
  `TinyMap<K, V>`; sparse subsets use `TinySecondaryMap<K, V>`.
- `TinyMap` generates keys while runtime image tables are materialized. Numeric
  casts, parallel counters, and stringify/reparse bridges do not allocate or
  translate keys.
- Stable-ID-to-runtime-key registries are private, table-local lowering state.
  They exist only long enough to resolve references into the runtime image.
- `cargo-boomerang` renders keys already assigned by lowering. Code generation
  does not independently choose runtime indices.
- Borrowed runtime views validate the concrete complete image consumed by an
  execution entry point. A view is not constructed merely to recover metadata
  already available from its image.

## Compiler-Side `TinyMap` Policy

The host compiler must construct runtime images, so `TinyMap` is valid in the
compiler crate when the value is runtime data or immediate image-building
state. Legitimate uses include:

- the dense tables owned by `OwnedEnclaveImage`;
- the Federate and Enclave tables owned by `OwnedCompiledDeployment`;
- the tables owned by `OwnedRtiImage`;
- temporary tables that directly become one of those owned image tables; and
- `RuntimeAssembly`, whose values are live runtime Enclaves rather than
  semantic compiler records.

It is not valid in semantic topology, deployment resolution, canonical
selection, or analysis structures. Nor should runtime keys be stored alongside
stable compiler records in a general-purpose compiler context. The distinction
is semantic ownership, not the crate or host process in which allocation
occurs.

## Enclave Lowering

Remove `EnclaveDomains` and separate Enclave lowering into two phases.

### Canonical Enclave Selection

A stable-ID-only selection gathers the canonically ordered Reactors, Actions,
Modes, representative Ports, and Reactions belonging to an Enclave. It may
borrow compiler records and store stable IDs, but it contains no runtime keys,
index spans, or dense registries.

Selection answers only which semantic records belong to the Enclave and in
what deterministic order.

### Runtime Image Materialization

A narrowly named Enclave image materializer consumes that selection and builds
`OwnedEnclaveImage`. It allocates final `TinyMap` tables in dependency order and
retains only the small stable-ID-to-key registries needed by subsequent table
builders.

Table construction remains decomposed into focused helpers for bindings,
routes, scopes, reactions, and packed relationship slices. Avoid replacing
`EnclaveDomains` with another omnibus context that carries every compiler
record and runtime registry. Each helper receives only the stable selection and
previously materialized tables or key registries that its references require.

Overflow errors arise from fallible `TinyMap` construction and packed-slice
builders, and are translated once into the existing resource-specific
`CompileError` variants.

## Complete Deployment Materialization

Add `OwnedCompiledDeployment::with_image`, which materializes a borrowed
`CompiledDeploymentImage` for the duration of a callback. It owns the temporary
rows needed to borrow stable strings and packed Enclave data without creating a
self-referential structure.

`OwnedCompiledDeployment::validate` delegates to `with_image` and constructs a
`CompiledDeploymentView` from the resulting image. This removes the existing
inline, validation-specific materialization path and gives direct owned
execution tests one canonical complete-image adapter.

This adapter is part of runtime image materialization. It does not change the
stable-ID boundary of topology, deployment resolution, or analysis.

## Federate Slice Simplification

Keep the compiler-side `FederateSlice`. It is a borrowed host projection used
by `cargo-boomerang` to select the Enclaves and metadata belonging to one
deployment-wide Federate.

Remove the runtime `FederateSliceImage` and `FederateSliceView`; they have no
independent runtime consumer. Also remove `FederateSlice::with_runtime_image`
and `FederateSlice::with_view`.

`cargo-boomerang` continues to render each selected `OwnedEnclaveImage` through
its short-lived `with_image` adapter. Generated root data is emitted directly:

- a selected `FederateIndex` constant;
- one `FederateImage` row;
- the one-row Federate table and federation-member table for local execution;
- the selected Enclave array.

The generated local `main` passes the selected Federate key directly to
`execute_owned_federate`. That function validates the complete
`CompiledDeploymentImage` before user initializers run. The distributed
placeholder reports the generated Federate identity without constructing a
runtime slice. A future distributed backend must define and validate the image
it actually consumes.

## Error Handling

- Canonical selection continues to return existing semantic validation and
  missing-reference errors.
- Dense table and packed-slice overflows are reported with the owning Enclave
  and resource name.
- `OwnedCompiledDeployment::with_image` is an infallible lifetime adapter;
  validation errors remain reported by `OwnedCompiledDeployment::validate`
  and runtime execution APIs.
- `federate_slice` continues to report an unknown Federate key or an invalid
  Enclave ownership span.

## Testing Strategy

Implementation must first use existing tests as its RED/GREEN seams. Extend or
adjust the closest existing assertion instead of adding a parallel test for the
new internal shape:

- `topology_debug_uses_stable_identity_not_dense_keys` protects stable compiler
  identities.
- `lowering_is_canonical_under_selection_reordering` protects deterministic
  image materialization and validates the resulting Enclave images.
- `owned_deployment_validates_the_complete_borrowed_hierarchy` exercises
  complete host-to-runtime image materialization and malformed root metadata;
  extend it to cover `OwnedCompiledDeployment::with_image`.
- `federate_slice_from_lowered_deployment_preserves_selected_root_rows`
  protects compiler-side Federate selection and exact range borrowing; remove
  only its runtime slice-wrapper assertions.
- Existing `cargo-boomerang` generated-source tests protect deployment-wide
  Federate and Enclave keys. Change their expectations from
  `FederateSliceImage` construction to direct root-table emission and direct
  execution with the selected key.
- `generated_monolith_matches_owned_reference_execution_summary` remains the
  end-to-end equivalence seam. Adapt its owned-reference helper to use the
  complete deployment image adapter.

Delete the slice-specific runtime validation tests together with
`FederateSliceImage` and `FederateSliceView`; do not recreate them under new
names. Their relevant ownership, ordering, range, and nested-image invariants
are already enforced by complete `CompiledDeploymentView` validation tests.

Add a new test only if a required externally observable invariant cannot be
expressed through one of these existing seams. Any such addition must state the
coverage gap it closes rather than mirror an implementation helper.

The reused seams must collectively demonstrate that compiler models retain
stable identities, runtime image tables generate and preserve typed keys,
equivalent input ordering produces identical images, generated launchers pass
the selected key directly, malformed complete images are rejected, and owned
and generated execution remain equivalent. Finish with offline workspace tests,
Clippy, formatting, and generated toolchain integrations.

## Non-goals

- Changing runtime scheduling or coordination semantics.
- Designing the future distributed backend injection API.
- Moving dense runtime keys into compiler topology or analysis models.
- Preserving compatibility for removed pre-release APIs.
