use std::hash::{Hash, Hasher};

#[cfg(feature = "federated")]
use std::collections::BTreeMap;

use boomerang_runtime::{ActionKey, EnclaveKey, PortKey, ReactionKey};
use boomerang_tinymap::TinySecondaryMap;

pub(crate) fn escape_entity_segment(segment: &str) -> String {
    segment.replace('\\', "\\\\").replace('/', "\\/")
}

/// Adapter-owned registration snapshot mirroring the lowered runtime execution shape.
#[derive(Debug)]
pub(super) enum RegistrationSnapshot {
    Local(TinySecondaryMap<EnclaveKey, EnclaveRegistration>),
    #[cfg(feature = "federated")]
    Federated(BTreeMap<String, TinySecondaryMap<EnclaveKey, EnclaveRegistration>>),
}

impl Default for RegistrationSnapshot {
    fn default() -> Self {
        Self::Local(TinySecondaryMap::new())
    }
}

#[derive(Debug)]
pub(super) struct EnclaveRegistration {
    scheduler: EntityRegistration,
    reactions: TinySecondaryMap<ReactionKey, EntityRegistration>,
    actions: TinySecondaryMap<ActionKey, EntityRegistration>,
    ports: TinySecondaryMap<PortKey, EntityRegistration>,
}

impl EnclaveRegistration {
    fn new(scheduler: EntityRegistration) -> Self {
        Self {
            scheduler,
            reactions: TinySecondaryMap::new(),
            actions: TinySecondaryMap::new(),
            ports: TinySecondaryMap::new(),
        }
    }
}

#[derive(Debug)]
pub(super) struct EntityRegistration {
    pub(super) path: String,
    pub(super) label: String,
}

impl EntityRegistration {
    fn new(path: String, label: String) -> Self {
        Self { path, label }
    }
}

impl RegistrationSnapshot {
    pub(super) fn resolve(
        &self,
        federate: Option<&str>,
        enclave: &str,
        kind: &'static str,
        identity: &str,
    ) -> Option<&EntityRegistration> {
        let enclave = enclave.parse::<EnclaveKey>().ok()?;
        let enclaves = match (self, federate) {
            (Self::Local(enclaves), None) => enclaves,
            #[cfg(feature = "federated")]
            (Self::Federated(federates), Some(federate)) => federates.get(federate)?,
            _ => return None,
        };
        let registration = enclaves.get(enclave)?;
        match kind {
            "scheduler" if identity == "scheduler" => Some(&registration.scheduler),
            "reaction" => registration
                .reactions
                .get(identity.parse::<ReactionKey>().ok()?),
            "action" => registration
                .actions
                .get(identity.parse::<ActionKey>().ok()?),
            "port" => registration.ports.get(identity.parse::<PortKey>().ok()?),
            _ => None,
        }
    }

    pub(super) fn local(enclaves: TinySecondaryMap<EnclaveKey, EnclaveRegistration>) -> Self {
        Self::Local(enclaves)
    }

    #[cfg(feature = "federated")]
    pub(super) fn federated(
        federates: BTreeMap<String, TinySecondaryMap<EnclaveKey, EnclaveRegistration>>,
    ) -> Self {
        Self::Federated(federates)
    }
}

pub(super) fn runtime_display_label(
    federate: Option<&str>,
    enclave: &str,
    display_name: &str,
    stable_key: &str,
) -> String {
    let mut parts = Vec::new();
    if let Some(federate) = federate {
        parts.push(bounded_fragment(federate, 12));
    }
    if !enclave.is_empty() {
        parts.push(compact_runtime_key(enclave));
    }
    let display = bounded_fragment(display_name, 24);
    let stable = compact_runtime_key(stable_key);
    parts.push(if display_name == stable_key {
        display
    } else {
        format!("{display}#{stable}")
    });
    bounded_fragment(&parts.join(" · "), 64)
}

pub(super) fn compact_runtime_key(value: &str) -> String {
    for (prefix, compact) in [
        ("EnclaveKey(", "E"),
        ("ReactorKey(", "Rr"),
        ("ReactionKey(", "R"),
        ("ActionKey(", "A"),
        ("PortKey(", "P"),
    ] {
        if let Some(index) = value
            .strip_prefix(prefix)
            .and_then(|value| value.strip_suffix(')'))
        {
            return format!("{compact}{index}");
        }
    }
    bounded_fragment(value, 16)
}

pub(super) fn bounded_fragment(value: &str, max_chars: usize) -> String {
    let sanitized = value.replace(['/', '\\'], "›");
    if sanitized.chars().count() <= max_chars {
        return sanitized;
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    let suffix = format!("~{:04x}", hasher.finish() as u16);
    let prefix = sanitized
        .chars()
        .take(max_chars.saturating_sub(suffix.chars().count()))
        .collect::<String>();
    format!("{prefix}{suffix}")
}

pub(super) fn log_runtime_enclaves(
    recording: &rerun::RecordingStream,
    federate: Option<&str>,
    enclaves: &boomerang_tinymap::TinyMap<
        boomerang_runtime::EnclaveKey,
        boomerang_runtime::Enclave,
    >,
) -> rerun::RecordingStreamResult<TinySecondaryMap<EnclaveKey, EnclaveRegistration>> {
    use boomerang_runtime::{ActionKey, PortKey, ReactionKey, ReactorKey};

    let mut registrations = TinySecondaryMap::new();
    for (enclave_key, enclave) in enclaves.iter() {
        let root = runtime_enclave_root(federate, enclave_key);
        let scheduler = format!("{root}/scheduler");
        let enclave_path = root.clone();
        let enclave_key_string = enclave_key.to_string();
        let enclave_label = runtime_display_label(
            federate,
            &enclave_key_string,
            &enclave_key_string,
            &enclave_key_string,
        );
        let scheduler_label =
            runtime_display_label(federate, &enclave_key_string, "scheduler", "scheduler");
        let mut registration = EnclaveRegistration::new(EntityRegistration::new(
            scheduler.clone(),
            scheduler_label.clone(),
        ));
        let mut nodes = vec![enclave_path.clone(), scheduler.clone()];
        let mut node_labels = vec![enclave_label, scheduler_label];
        let mut edges = vec![(enclave_path.clone(), scheduler.clone(), "owns_scheduler")];

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
            node_labels.push(runtime_display_label(
                federate,
                &enclave_key_string,
                reactor.name().rsplit('/').next().unwrap_or(reactor.name()),
                &key.to_string(),
            ));
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
            let label = runtime_display_label(
                federate,
                &enclave_key_string,
                reaction.get_name(),
                &key.to_string(),
            );
            nodes.push(path.clone());
            node_labels.push(label.clone());
            edges.push((owner_path, path.clone(), "owns_reaction"));
            let owner_key = owner.to_string();
            registration
                .reactions
                .insert(key, EntityRegistration::new(path.clone(), label));
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
            let label = runtime_display_label(
                federate,
                &enclave_key_string,
                action.name(),
                &key.to_string(),
            );
            nodes.push(path.clone());
            node_labels.push(label.clone());
            edges.push((reactor_path(owner), path.clone(), "owns_action"));
            let owner_key = owner.to_string();
            registration
                .actions
                .insert(key, EntityRegistration::new(path.clone(), label));
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
            let label = runtime_display_label(
                federate,
                &enclave_key_string,
                port.get_name(),
                &key.to_string(),
            );
            nodes.push(path.clone());
            node_labels.push(label.clone());
            edges.push((reactor_path(owner), path.clone(), "owns_port"));
            let owner_key = owner.to_string();
            registration
                .ports
                .insert(key, EntityRegistration::new(path.clone(), label));
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
            &rerun::GraphNodes::new(nodes.clone()).with_labels(node_labels),
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
        registrations.insert(enclave_key, registration);
    }
    Ok(registrations)
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
