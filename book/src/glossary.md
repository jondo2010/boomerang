# Glossary

This glossary defines the main terms used by Boomerang's public API and
documentation. In type names, suffixes such as `Spec`, `Context`, and `Factory`
describe where a value belongs in the lifecycle from application declaration to
runtime execution.

## Action

A typed event owned by a reactor. A logical action is scheduled in logical time;
a physical action introduces an event from outside the deterministic reactor
graph. `TypedActionKey<T, Q>` identifies an action while retaining its payload
and action-kind types.

## Assembly

The complete build-time model of a Boomerang application. `Assembly` stores the
declared reactor, reaction, action, port, mode, and connection specifications,
validates their relationships, analyzes dependencies and partitions, and then
lowers the model into a `RuntimeAssembly`.

The published crate remains named `boomerang_builder`, and the top-level facade
continues to expose it as `boomerang::builder`. Within that package, however,
`Assembly` is the name for the build-time graph rather than “environment” or a
generic builder.

## Assembly Error

`AssemblyError` reports failures while declaring, validating, or lowering an
assembly. Examples include duplicate definitions, invalid connections,
dependency cycles, unsupported federation topology, and unresolved assembly
keys.

## Assembly Fully Qualified Name

`AssemblyFqn` is a hierarchical name for an object in an assembly, such as a
reactor, action, reaction, or port. It is useful for lookup and diagnostics.
An assembly FQN describes the declared graph; it is not yet the durable logical
identity promised by the future deployment-independent recording model.

## Assembly Key

A slot-map identity for one declaration stored in an `Assembly`. The concrete
types are `AssemblyReactorKey`, `AssemblyReactionKey`, `AssemblyActionKey`,
`AssemblyPortKey`, and `AssemblyModeKey`.

Assembly keys are valid while constructing and lowering that assembly. They are
not stable recording identifiers and must not be persisted as
deployment-independent identity. Typed wrappers such as `TypedActionKey`,
`TypedPortKey`, and `TimerActionKey` add domain and payload information around
the corresponding assembly key.

## Boundary

A connection point between runtime partitions. A boundary may be local between
enclaves or federated between coordinated participants. `BoundaryKind` and
`InterPartitionPlan` describe the result of boundary analysis during lowering;
the runtime backend supplies the corresponding delivery mechanism.

## Connection

A declared route from an output port to an input port. A connection may have a
logical delay and may remain within an enclave or cross an enclave or federate
boundary. Lowering chooses the runtime delivery mechanism while preserving the
connection's logical behavior.

## Context

A temporary cursor used while declaring part of an assembly. `ReactorContext`
is the primary example: reactor macro output and manual declaration code use it
to add ports, actions, modes, child reactors, reactions, and connections. A
context mutates the assembly; it is not a stored graph node and does not survive
lowering. Local variables conventionally use the short name `ctx`.

## Declaration

A fluent, temporary API that records a specification in an assembly.
`ReactionDeclaration` collects a reaction's triggers, uses, effects, mode scope,
and function before `finish` records a `ReactionSpec`. This is distinct from the
stored specification and from a factory that creates a runtime object later.

## Enclave

A runtime scheduling partition. Reactors within one enclave share a scheduler
and can use direct runtime relationships. Connections between enclaves require
asynchronous boundary delivery. An enclave is a runtime execution boundary; it
is not synonymous with a federate, process, or host.

## Factory

A callable value that creates a runtime object once lowering has resolved the
runtime keys and aliases it needs. The `Factory` suffix distinguishes deferred
runtime creation from assembly declaration. Examples include
`ActionFactoryFn`, `DeferredReactionFactory`, and the `DeferredRuntimeFactory`
trait.

Factories may capture declaration-time configuration, but they run at the
assembly-to-runtime boundary rather than adding new specifications to the
assembly.

## Federate

A statically identified participant in coordinated federated execution. In the
current experimental federation slice, each federate is placed at an enclave
root and communicates through the runtime infrastructure loop (RTI). Federates,
enclaves, processes, and hosts are separate concepts even where the current
runner maps them one-to-one. See [Static Federation](./static-federation.md).

## History Transition

A mode transition that preserves the target mode's local timing state. Pending
mode-local logical actions, timers, and delayed connections resume with the same
remaining local delay they had when the mode became inactive.

## Local Time

Logical time measured only while a mode scope is active. Mode-local timers,
logical actions, and delayed connections use local time.

## Logical Time

The deterministic time coordinate used to order Boomerang events independently
of wall-clock scheduling. A `Tag` contains a timestamp offset and a microstep;
microsteps distinguish ordered events at the same timestamp. Logical actions,
timers, delayed connections, federation messages, and recordings must preserve
the complete tag.

## Lowering

The consuming pass that transforms an `Assembly` into executable runtime data.
`Assembly::into_runtime_assembly` validates the graph, chooses partitions,
allocates runtime reactors, actions, ports, modes, and reactions, resolves
assembly keys to runtime keys, and produces a `RuntimeAssembly`.

Lowering is distinct from declaration: declaration records the logical graph;
lowering materializes a particular runtime representation of it.

## Mode

A named state of a reactor. Exactly one sibling mode is active at a logical
instant. `ModeSpec` stores a declared mode, while `ModeEffectSpec` describes a
reset or history transition requested as a reaction effect.

## Mode Scope

The static region of a reactor contained by a mode. Reactions, timers, logical
actions, child reactors, and delayed connections declared inside the mode belong
to that scope.

## Port

A typed data endpoint on a reactor. Reactions read input ports and write output
ports; connections route values between compatible ports.
`TypedPortKey<T, Q, A>` retains the payload type, direction, and locality
information used during safe declaration.

## Partition

A region of the declared graph selected to execute behind one runtime boundary.
Current lowering maps each partition root to an enclave and records
cross-partition connections in an `InterPartitionPlan`. A partition is a graph
and deployment concept; it is not automatically a process, host, or federate.

## Reaction

A deterministic unit of behavior that runs when one of its triggers is present.
A reaction may read declared uses and write declared effects. Its declaration is
collected through `ReactionDeclaration`, stored as `ReactionSpec`, and lowered
to a runtime reaction function.

## Reactor

The main compositional component in Boomerang. A reactor owns state, ports,
actions, reactions, modes, and child reactors. The `Reactor` trait is the
application-facing construction interface commonly implemented by generated
macro code; `ReactorSpec` is the type-erased declaration stored in an assembly.

## Reactor Placement

`ReactorPlacement` records whether a declared reactor remains local, begins a
new enclave, or represents a federate. Placement influences partition analysis
during lowering without changing the reactor's logical behavior.

## Reset Transition

The default mode transition. It enters the target mode with fresh local timing
state, discards pending mode-local events in the reset scope, and returns
contained modal child reactors to their initial modes.

## Runtime Aliases

`RuntimeAliases` maps assembly keys to the runtime keys allocated during
lowering. Deferred factories use these maps when they need the runtime identity
of an object declared earlier. These aliases are runtime-construction data, not
durable application identity.

## Runtime Assembly

`RuntimeAssembly` is the ready-to-run result of lowering an `Assembly`. It owns
runtime enclaves and the resolved metadata needed by features such as replay and
static federation. Runners consume it to create schedulers or federation roles;
it no longer accepts logical graph declarations.

## Runtime Environment

`boomerang_runtime::Env` is the executable state owned by a runtime enclave,
including runtime reactors, actions, ports, reactions, and dependency data. It
is genuine runtime “environment” vocabulary and is distinct from the build-time
`Assembly`. Runners may return final runtime environments for inspection after
execution.

## Specification (`Spec`)

A stored build-time declaration inside an `Assembly`. `ReactorSpec`,
`ReactionSpec`, `ActionSpec`, `PortSpec`, `ModeSpec`, and `ConnectionSpec`
describe the logical graph before runtime allocation. `TimerSpec`,
`ModeEffectSpec`, and `FederateSpec` are focused configuration specifications.

A `Spec` is data that the assembly validates and lowers. It is not the temporary
context or declaration API used to record that data, and it is not the runtime
object produced afterward. Traits such as `ErasedPortSpec`,
`ErasedConnectionSpec`, and `ParentReactorSpec` provide type-erased or shared
views over stored specifications.

## Tag

The concrete logical-time coordinate of an event: a timestamp offset plus a
microstep. Tags provide deterministic ordering, including multiple causally
ordered events at the same timestamp. Recording, replay, and federation must
preserve both components.

## Timer

A built-in logical action scheduled from a `TimerSpec`. A timer may have an
initial offset and an optional period. `TimerActionKey` is the typed declaration
key used to trigger reactions from that timer.
