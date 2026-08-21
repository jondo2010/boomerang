use std::collections::HashMap;

pub(crate) fn escape_entity_segment(segment: &str) -> String {
    segment.replace('\\', "\\\\").replace('/', "\\/")
}

/// Compact adapter-owned lookup derived synchronously from static registration.
#[derive(Clone, Debug, Default)]
pub(super) struct RegistrationIndex {
    entities: HashMap<RegistrationLookup, RegistrationResolution>,
    federated_entities: HashMap<FederatedRegistrationLookup, RegistrationResolution>,
    action_triggers: HashMap<String, Vec<String>>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct RegistrationLookup {
    federate: Option<String>,
    enclave: String,
    kind: &'static str,
    identity: String,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct FederatedRegistrationLookup {
    federate: String,
    kind: &'static str,
    identity: String,
}

#[derive(Clone, Debug)]
enum RegistrationResolution {
    Unique(String),
    Ambiguous,
}

impl RegistrationIndex {
    pub(super) fn register_in_federate(
        &mut self,
        federate: Option<&str>,
        enclave: &str,
        kind: &'static str,
        stable_key: &str,
        display_name: &str,
        path: &str,
    ) {
        self.register_identity(federate, enclave, kind, stable_key, path);
        if display_name != stable_key {
            self.register_identity(federate, enclave, kind, display_name, path);
        }
    }

    pub(super) fn register_action_trigger(&mut self, action_path: &str, reaction_path: &str) {
        let reactions = self
            .action_triggers
            .entry(action_path.to_owned())
            .or_default();
        if !reactions.iter().any(|reaction| reaction == reaction_path) {
            reactions.push(reaction_path.to_owned());
        }
    }

    fn register_identity(
        &mut self,
        federate: Option<&str>,
        enclave: &str,
        kind: &'static str,
        identity: &str,
        path: &str,
    ) {
        let lookup = RegistrationLookup {
            federate: federate.map(str::to_owned),
            enclave: enclave.to_owned(),
            kind,
            identity: identity.to_owned(),
        };
        register_resolution(&mut self.entities, lookup, path);
        if let Some(federate) = federate {
            register_resolution(
                &mut self.federated_entities,
                FederatedRegistrationLookup {
                    federate: federate.to_owned(),
                    kind,
                    identity: identity.to_owned(),
                },
                path,
            );
        }
    }

    pub(super) fn resolve(
        &self,
        federate: Option<&str>,
        enclave: &str,
        kind: &'static str,
        identity: &str,
    ) -> Option<String> {
        resolve_registration(
            &self.entities,
            &RegistrationLookup {
                federate: federate.map(str::to_owned),
                enclave: enclave.to_owned(),
                kind,
                identity: identity.to_owned(),
            },
        )
    }

    pub(super) fn resolve_federated(
        &self,
        federate: &str,
        kind: &'static str,
        identity: &str,
    ) -> Option<String> {
        resolve_registration(
            &self.federated_entities,
            &FederatedRegistrationLookup {
                federate: federate.to_owned(),
                kind,
                identity: identity.to_owned(),
            },
        )
    }

    pub(super) fn triggered_reactions(&self, action_path: &str) -> &[String] {
        self.action_triggers
            .get(action_path)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub(super) fn action_triggers_reaction(
        &self,
        federate: Option<&str>,
        enclave: &str,
        action: &str,
        reaction_path: &str,
    ) -> Option<bool> {
        self.resolve(federate, enclave, "action", action)
            .map(|action_path| {
                self.triggered_reactions(&action_path)
                    .iter()
                    .any(|path| reaction_path.starts_with(path))
            })
    }

    pub(super) fn merge(&mut self, other: Self) {
        for (lookup, incoming) in other.entities {
            merge_resolution(&mut self.entities, lookup, incoming);
        }
        for (lookup, incoming) in other.federated_entities {
            merge_resolution(&mut self.federated_entities, lookup, incoming);
        }
        for (action, reactions) in other.action_triggers {
            let existing = self.action_triggers.entry(action).or_default();
            for reaction in reactions {
                if !existing.contains(&reaction) {
                    existing.push(reaction);
                }
            }
        }
    }
}

fn register_resolution<K: Eq + std::hash::Hash>(
    map: &mut HashMap<K, RegistrationResolution>,
    key: K,
    path: &str,
) {
    merge_resolution(map, key, RegistrationResolution::Unique(path.to_owned()));
}

fn merge_resolution<K: Eq + std::hash::Hash>(
    map: &mut HashMap<K, RegistrationResolution>,
    key: K,
    incoming: RegistrationResolution,
) {
    map.entry(key)
        .and_modify(|current| {
            if !matches!(
                (&*current, &incoming),
                (RegistrationResolution::Unique(left), RegistrationResolution::Unique(right))
                    if left == right
            ) {
                *current = RegistrationResolution::Ambiguous;
            }
        })
        .or_insert(incoming);
}

fn resolve_registration<K: Eq + std::hash::Hash>(
    map: &HashMap<K, RegistrationResolution>,
    key: &K,
) -> Option<String> {
    match map.get(key) {
        Some(RegistrationResolution::Unique(path)) => Some(path.clone()),
        Some(RegistrationResolution::Ambiguous) | None => None,
    }
}

pub(super) fn log_runtime_enclaves(
    recording: &rerun::RecordingStream,
    federate: Option<&str>,
    enclaves: &boomerang_tinymap::TinyMap<
        boomerang_runtime::EnclaveKey,
        boomerang_runtime::Enclave,
    >,
    index: &mut RegistrationIndex,
) -> rerun::RecordingStreamResult<()> {
    use boomerang_runtime::{ActionKey, PortKey, ReactionKey, ReactorKey};

    for (enclave_key, enclave) in enclaves.iter() {
        let root = runtime_enclave_root(federate, enclave_key);
        let scheduler = format!("{root}/scheduler");
        let enclave_path = root.clone();
        let mut nodes = vec![enclave_path.clone(), scheduler.clone()];
        let mut edges = vec![(enclave_path.clone(), scheduler.clone(), "owns_scheduler")];
        let enclave_key_string = enclave_key.to_string();
        index.register_in_federate(
            federate,
            &enclave_key_string,
            "scheduler",
            "scheduler",
            "scheduler",
            &scheduler,
        );

        log_runtime_entity(
            recording,
            &enclave_path,
            &enclave_key.to_string(),
            &enclave_key.to_string(),
            "enclave",
            &[
                ("boomerang.runtime.owner_key", federate),
                (
                    "boomerang.runtime.type",
                    Some(std::any::type_name::<boomerang_runtime::Enclave>()),
                ),
            ],
        )?;
        log_runtime_entity(
            recording,
            &scheduler,
            "scheduler",
            "scheduler",
            "scheduler",
            &[
                (
                    "boomerang.runtime.owner_key",
                    Some(&enclave_key.to_string()),
                ),
                (
                    "boomerang.runtime.type",
                    Some(std::any::type_name::<boomerang_runtime::Scheduler>()),
                ),
            ],
        )?;

        let reactor_path = |key: ReactorKey| runtime_reactor_path(&root, enclave, key);

        for (key, reactor) in enclave.env.reactors.iter() {
            let path = reactor_path(key);
            let owner = enclave
                .graph
                .reactor_root_scopes
                .get(key)
                .and_then(|scope| {
                    enclave.graph.scopes[*scope]
                        .parent
                        .map(|parent| enclave.graph.scopes[parent].reactor)
                        .filter(|parent| *parent != key)
                });
            let owner_path = owner
                .map(&reactor_path)
                .unwrap_or_else(|| enclave_path.clone());
            nodes.push(path.clone());
            edges.push((owner_path, path.clone(), "owns_reactor"));
            let owner_key = owner
                .map(|owner| owner.to_string())
                .unwrap_or_else(|| enclave_key.to_string());
            log_runtime_entity(
                recording,
                &path,
                reactor.name().rsplit('/').next().unwrap_or(reactor.name()),
                &key.to_string(),
                "reactor",
                &[("boomerang.runtime.owner_key", Some(&owner_key))],
            )?;
        }

        let reaction_levels = reaction_levels(&enclave.graph);
        for (key, reaction) in enclave.env.reactions.iter() {
            let owner = enclave.graph.reaction_reactors[key];
            let path = format!(
                "{}/reactions/{}",
                reactor_path(owner),
                escape_entity_segment(&key.to_string())
            );
            let owner_path = reactor_path(owner);
            nodes.push(path.clone());
            edges.push((owner_path, path.clone(), "owns_reaction"));
            let owner_key = owner.to_string();
            index.register_in_federate(
                federate,
                &enclave_key_string,
                "reaction",
                &key.to_string(),
                reaction.get_name(),
                &path,
            );
            log_runtime_entity(
                recording,
                &path,
                reaction.get_name(),
                &key.to_string(),
                "reaction",
                &[
                    ("boomerang.runtime.owner_key", Some(&owner_key)),
                    (
                        "boomerang.runtime.reaction_level",
                        reaction_levels
                            .get(&key)
                            .map(ToString::to_string)
                            .as_deref(),
                    ),
                ],
            )?;
        }

        for (key, action) in enclave.env.actions.iter() {
            let owner = owner_reactor_for_action(&enclave.graph, key);
            let path = format!(
                "{}/actions/{}",
                reactor_path(owner),
                escape_entity_segment(&key.to_string())
            );
            nodes.push(path.clone());
            edges.push((reactor_path(owner), path.clone(), "owns_action"));
            let owner_key = owner.to_string();
            index.register_in_federate(
                federate,
                &enclave_key_string,
                "action",
                &key.to_string(),
                action.name(),
                &path,
            );
            log_runtime_entity(
                recording,
                &path,
                action.name(),
                &key.to_string(),
                "action",
                &[
                    ("boomerang.runtime.owner_key", Some(&owner_key)),
                    ("boomerang.runtime.type", Some(action.type_name())),
                    (
                        "boomerang.runtime.action_timing",
                        Some(if action.is_logical() {
                            "logical"
                        } else {
                            "physical"
                        }),
                    ),
                ],
            )?;
        }

        for (key, port) in enclave.env.ports.iter() {
            let owner = owner_reactor_for_port(&enclave.graph, key);
            let path = format!(
                "{}/ports/{}",
                reactor_path(owner),
                escape_entity_segment(&key.to_string())
            );
            nodes.push(path.clone());
            edges.push((reactor_path(owner), path.clone(), "owns_port"));
            let owner_key = owner.to_string();
            index.register_in_federate(
                federate,
                &enclave_key_string,
                "port",
                &key.to_string(),
                port.get_name(),
                &path,
            );
            log_runtime_entity(
                recording,
                &path,
                port.get_name(),
                &key.to_string(),
                "port",
                &[
                    ("boomerang.runtime.owner_key", Some(&owner_key)),
                    ("boomerang.runtime.type", Some(port.type_name())),
                ],
            )?;
        }

        let reaction_path = |key: ReactionKey| {
            let owner = enclave.graph.reaction_reactors[key];
            format!(
                "{}/reactions/{}",
                reactor_path(owner),
                escape_entity_segment(&key.to_string())
            )
        };
        let action_path = |key: ActionKey| {
            let owner = owner_reactor_for_action(&enclave.graph, key);
            format!(
                "{}/actions/{}",
                reactor_path(owner),
                escape_entity_segment(&key.to_string())
            )
        };
        let port_path = |key: PortKey| {
            let owner = owner_reactor_for_port(&enclave.graph, key);
            format!(
                "{}/ports/{}",
                reactor_path(owner),
                escape_entity_segment(&key.to_string())
            )
        };

        for (action, reactions) in enclave.graph.action_triggers.iter() {
            let action_path = action_path(action);
            for (_, reaction) in reactions {
                let reaction = reaction_path(*reaction);
                index.register_action_trigger(&action_path, &reaction);
            }
            edges.extend(
                reactions.iter().map(|(_, reaction)| {
                    (action_path.clone(), reaction_path(*reaction), "triggers")
                }),
            );
        }
        for (port, reactions) in enclave.graph.port_triggers.iter() {
            edges.extend(
                reactions
                    .iter()
                    .map(|(_, reaction)| (port_path(port), reaction_path(*reaction), "triggers")),
            );
        }
        for (reaction, ports) in enclave.graph.reaction_use_ports.iter() {
            edges.extend(
                ports
                    .iter()
                    .map(|port| (port_path(*port), reaction_path(reaction), "uses")),
            );
        }
        for (reaction, ports) in enclave.graph.reaction_effect_ports.iter() {
            edges.extend(
                ports
                    .iter()
                    .map(|port| (reaction_path(reaction), port_path(*port), "effects")),
            );
        }
        for (reaction, actions) in enclave.graph.reaction_actions.iter() {
            edges.extend(actions.iter().map(|action| {
                (
                    reaction_path(reaction),
                    action_path(*action),
                    "action_use_or_effect",
                )
            }));
        }
        for (downstream, _) in enclave.downstream_enclaves.iter() {
            edges.push((
                scheduler.clone(),
                format!("{}/scheduler", runtime_enclave_root(federate, downstream)),
                "scheduler_coordination",
            ));
        }

        for (index, (source, target, kind)) in edges.iter().enumerate() {
            log_runtime_relation(
                recording,
                &format!("{root}/topology/relations/{index}"),
                source,
                target,
                kind,
                None,
                None,
            )?;
        }

        recording.log_static(
            format!("{root}/topology"),
            &rerun::GraphNodes::new(nodes.clone()).with_labels(nodes),
        )?;
        recording.log_static(
            format!("{root}/topology"),
            &rerun::GraphEdges::new(
                edges
                    .iter()
                    .map(|(source, target, _)| (source.as_str(), target.as_str())),
            )
            .with_graph_type(rerun::components::GraphType::Directed),
        )?;
    }
    Ok(())
}

fn runtime_reactor_path(
    root: &str,
    enclave: &boomerang_runtime::Enclave,
    key: boomerang_runtime::ReactorKey,
) -> String {
    let mut key_chain = Vec::new();
    let mut current = Some(key);
    while let Some(reactor_key) = current {
        key_chain.push(reactor_key);
        current = enclave
            .graph
            .reactor_root_scopes
            .get(reactor_key)
            .and_then(|scope| enclave.graph.scopes[*scope].parent)
            .map(|parent| enclave.graph.scopes[parent].reactor)
            .filter(|parent| *parent != reactor_key);
    }
    key_chain.reverse();

    let first_fqn = enclave.env.reactors[key_chain[0]].name();
    let mut hierarchy = first_fqn
        .split('/')
        .map(escape_entity_segment)
        .collect::<Vec<_>>();
    hierarchy.pop();
    hierarchy.extend(key_chain.into_iter().map(|reactor_key| {
        let reactor = &enclave.env.reactors[reactor_key];
        let display_name = reactor.name().rsplit('/').next().unwrap_or(reactor.name());
        format!(
            "{}@{}",
            escape_entity_segment(display_name),
            escape_entity_segment(&reactor_key.to_string())
        )
    }));
    format!("{root}/reactors/{}", hierarchy.join("/reactors/"))
}

pub(super) fn log_runtime_relation(
    recording: &rerun::RecordingStream,
    path: &str,
    source: &str,
    target: &str,
    kind: &str,
    stable_key: Option<&str>,
    delay_ns: Option<u64>,
) -> rerun::RecordingStreamResult<()> {
    let mut relation = rerun::DynamicArchetype::new("boomerang.RuntimeRelation")
        .with_component::<rerun::components::Text>("boomerang.runtime.source", [source])
        .with_component::<rerun::components::Text>("boomerang.runtime.target", [target])
        .with_component::<rerun::components::Text>("boomerang.runtime.relation_kind", [kind]);
    if let Some(stable_key) = stable_key {
        relation = relation.with_component::<rerun::components::Text>(
            "boomerang.runtime.stable_key",
            [stable_key],
        );
    }
    if let Some(delay_ns) = delay_ns {
        relation = relation.with_component_from_data(
            "boomerang.runtime.delay_ns",
            std::sync::Arc::new(rerun::external::arrow::array::UInt64Array::from(vec![
                delay_ns,
            ])),
        );
    }
    recording.log_static(path, &relation)
}

fn log_runtime_entity(
    recording: &rerun::RecordingStream,
    path: &str,
    display_name: &str,
    stable_key: &str,
    kind: &str,
    optional_components: &[(&'static str, Option<&str>)],
) -> rerun::RecordingStreamResult<()> {
    let mut entity = rerun::DynamicArchetype::new("boomerang.RuntimeEntity")
        .with_component::<rerun::components::Text>("boomerang.runtime.display_name", [display_name])
        .with_component::<rerun::components::Text>("boomerang.runtime.stable_key", [stable_key])
        .with_component::<rerun::components::Text>("boomerang.runtime.kind", [kind]);
    for (name, value) in optional_components {
        if let Some(value) = value {
            entity = entity.with_component::<rerun::components::Text>(*name, [*value]);
        }
    }
    recording.log_static(path, &entity)
}

pub(super) fn runtime_enclave_root(
    federate: Option<&str>,
    enclave: boomerang_runtime::EnclaveKey,
) -> String {
    let enclave = escape_entity_segment(&enclave.to_string());
    match federate {
        Some(federate) => format!(
            "/federates/{}/enclaves/{enclave}",
            escape_entity_segment(federate)
        ),
        None => format!("/enclaves/{enclave}"),
    }
}

fn owner_reactor_for_action(
    graph: &boomerang_runtime::ReactionGraph,
    action: boomerang_runtime::ActionKey,
) -> boomerang_runtime::ReactorKey {
    graph.scopes[graph.action_scopes[action]].reactor
}

fn owner_reactor_for_port(
    graph: &boomerang_runtime::ReactionGraph,
    port: boomerang_runtime::PortKey,
) -> boomerang_runtime::ReactorKey {
    graph.scopes[graph.port_scopes[port]].reactor
}

fn reaction_levels(
    graph: &boomerang_runtime::ReactionGraph,
) -> std::collections::BTreeMap<boomerang_runtime::ReactionKey, boomerang_runtime::Level> {
    let mut levels = std::collections::BTreeMap::new();
    for (level, reaction) in graph
        .action_triggers
        .values()
        .flatten()
        .chain(graph.port_triggers.values().flatten())
        .chain(graph.reset_reactions.values().flatten())
        .chain(
            graph
                .startup_reactions
                .values()
                .flatten()
                .map(|entry| &entry.reaction),
        )
        .chain(
            graph
                .shutdown_reactions_by_scope
                .values()
                .flatten()
                .map(|entry| &entry.reaction),
        )
    {
        levels.insert(*reaction, *level);
    }
    levels
}
