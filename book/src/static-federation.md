# Static Federation

Boomerang has an experimental `federated` feature for static federated
reactors. A federate is a reactor instance placed behind
`add_child_federate`; cross-federate logical messages are serialized with a
registered payload codec and coordinated by a runtime infrastructure loop
(RTI).

The in-memory and TCP runners execute persistent static federates with the same
logical-time scheduler hooks used by the protocol client. A typical setup
registers a codec, builds runtime parts, and then selects a runner:

```rust,ignore
let mut env_builder = EnvBuilder::new();
env_builder.register_federated_codec::<u32, _>(boomerang::federated::SerdeJsonCodec)?;
let config = runtime::Config::default().with_fast_forward(true);
let parts = env_builder.into_runtime_parts(&config)?;
let envs = execute_federation_in_memory(parts, config)?;
```

The TCP runner is also synchronous and single-process. It starts a static RTI
listener, connects every federate scheduler through the shared TCP protocol
transport, and returns the same final runtime environments:

```rust,ignore
let config = runtime::Config::default().with_fast_forward(true);
let parts = env_builder.into_runtime_parts(&config)?;
let envs = execute_federation_over_tcp(
    parts,
    config,
    TcpStaticFederationConfig::default(),
)?;
```

The default TCP configuration binds `127.0.0.1:0`, so the operating system
selects an unused localhost port. This runner proves real framed transport; it
does not launch separate processes or provide dynamic federation membership.

The supported subset is deliberately conservative. It supports static
persistent federates, one runtime enclave per federate, logical
cross-federate messages routed through the RTI, same-tag messages,
same-timestamp microsteps, fanout, multi-hop topologies, shutdown/no-future
coordination, and positive-delay distributed cycles.

The implementation rejects cross-federate physical connections, transient
federates, mixed local/federated boundaries, and distributed zero-delay cycles.
It does not implement `PTAG` or `ABS`, dynamic federate join/leave, reconnect
behavior, authentication, or direct federate-to-federate payload channels.

Run the public in-memory federation proof with:

```sh
cargo test -p boomerang --features federated public_api_runs_static_in_memory_federation
```

Run the ignored localhost TCP proof with:

```sh
cargo test -p boomerang --features federated tcp_static -- --ignored
```

If a sandbox reports `Operation not permitted` while binding localhost, rerun
that focused command with socket permission. The failure is environmental; the
non-network in-memory tests remain the primary correctness suite.
