use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet, BinaryHeap},
};

use petgraph::{
    algo::{kosaraju_scc, toposort},
    stable_graph::{NodeIndex, StableDiGraph},
    visit::{EdgeRef, IntoEdgeReferences},
};

use super::{BoundaryId, FederateId};

/// Nonnegative logical delay carried by a federation endpoint.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct FederationDelay(
    /// Delay represented in nanoseconds.
    u64,
);

impl FederationDelay {
    /// Constructs a delay from its nonnegative nanosecond representation.
    #[must_use]
    pub const fn from_nanos(nanoseconds: u64) -> Self {
        Self(nanoseconds)
    }

    /// Returns this delay in nanoseconds.
    #[must_use]
    pub const fn as_nanos(self) -> u64 {
        self.0
    }
}

/// One preserved cross-federate endpoint between declared members.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FederationEndpoint {
    /// Declared source member identity.
    source: FederateId,
    /// Declared target member identity.
    target: FederateId,
    /// Stable endpoint identity.
    id: BoundaryId,
    /// Direct delay applied by this endpoint.
    delay: FederationDelay,
}

impl FederationEndpoint {
    /// Constructs a resolved cross-federate endpoint.
    #[must_use]
    pub fn new(
        source: FederateId,
        target: FederateId,
        id: BoundaryId,
        delay: FederationDelay,
    ) -> Self {
        Self {
            source,
            target,
            id,
            delay,
        }
    }

    /// Returns the declared source member.
    #[must_use]
    pub fn source(&self) -> &FederateId {
        &self.source
    }

    /// Returns the declared target member.
    #[must_use]
    pub fn target(&self) -> &FederateId {
        &self.target
    }

    /// Returns the stable endpoint identity.
    #[must_use]
    pub fn id(&self) -> &BoundaryId {
        &self.id
    }

    /// Returns the endpoint's direct delay.
    #[must_use]
    pub const fn delay(&self) -> FederationDelay {
        self.delay
    }
}

/// Reports invalid or unrepresentable federation graph analysis inputs.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum FederationAnalysisError {
    /// A member identity was declared more than once.
    #[error("duplicate federate identity '{federate}'")]
    DuplicateFederateId {
        /// Conflicting member identity.
        federate: FederateId,
    },
    /// An endpoint identity was declared more than once.
    #[error("duplicate federation endpoint identity '{endpoint}'")]
    DuplicateEndpointId {
        /// Conflicting endpoint identity.
        endpoint: BoundaryId,
    },
    /// An endpoint references a member absent from the declared member set.
    #[error("federation endpoint '{endpoint}' references undeclared federate '{federate}'")]
    UndeclaredEndpointMember {
        /// Endpoint carrying the unresolved member reference.
        endpoint: BoundaryId,
        /// Member identity absent from the declared member set.
        federate: FederateId,
    },
    /// A cycle whose every endpoint has zero delay would prevent progress.
    #[error("zero-delay federation cycle among {federates:?}")]
    ZeroDelayCycle {
        /// Canonically ordered member identities in the first invalid cycle component.
        federates: Vec<FederateId>,
    },
    /// The minimum accumulated delay cannot be represented in `u64` nanoseconds.
    #[error("minimum federation delay from '{from}' to '{target}' exceeds u64 nanoseconds")]
    AccumulatedDelayOverflow {
        /// Source member of the unrepresentable path.
        from: FederateId,
        /// Target member of the unrepresentable path.
        target: FederateId,
    },
}

/// Canonical, backend-neutral analysis of declared federation members and endpoints.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnalyzedFederationGraph {
    /// Canonically ordered declared member identities.
    members: Vec<FederateId>,
    /// Canonically ordered endpoint records, including parallel endpoints.
    endpoints: Vec<FederationEndpoint>,
    /// Minimum direct incoming dependency by target then source member.
    direct_incoming: BTreeMap<FederateId, Vec<(FederateId, FederationDelay)>>,
    /// Minimum non-empty incoming path by target then source member.
    transitive_incoming: BTreeMap<FederateId, Vec<(FederateId, FederationDelay)>>,
    /// Reachable downstream members by source member, excluding itself.
    affected_downstream: BTreeMap<FederateId, Vec<FederateId>>,
}

impl AnalyzedFederationGraph {
    /// Returns declared members in canonical identity order.
    #[must_use]
    pub fn members(&self) -> &[FederateId] {
        &self.members
    }

    /// Returns preserved endpoints in canonical source, target, identity, and delay order.
    #[must_use]
    pub fn endpoints(&self) -> &[FederationEndpoint] {
        &self.endpoints
    }

    /// Returns minimum direct incoming dependencies for a declared target member.
    #[must_use]
    pub fn direct_incoming(&self, target: &FederateId) -> Option<&[(FederateId, FederationDelay)]> {
        self.direct_incoming.get(target).map(Vec::as_slice)
    }

    /// Returns minimum non-empty incoming paths for a declared target member.
    #[must_use]
    pub fn transitive_incoming(
        &self,
        target: &FederateId,
    ) -> Option<&[(FederateId, FederationDelay)]> {
        self.transitive_incoming.get(target).map(Vec::as_slice)
    }

    /// Returns reachable downstream members for a declared source member.
    #[must_use]
    pub fn affected_downstream(&self, source: &FederateId) -> Option<&[FederateId]> {
        self.affected_downstream.get(source).map(Vec::as_slice)
    }
}

/// Validates and canonically analyzes explicit federation members and endpoints.
pub fn analyze_federation_graph(
    members: impl IntoIterator<Item = FederateId>,
    endpoints: impl IntoIterator<Item = FederationEndpoint>,
) -> Result<AnalyzedFederationGraph, FederationAnalysisError> {
    let mut member_set = BTreeSet::new();
    for member in members {
        if !member_set.insert(member.clone()) {
            return Err(FederationAnalysisError::DuplicateFederateId { federate: member });
        }
    }
    let members = member_set.into_iter().collect::<Vec<_>>();

    let mut endpoint_ids = BTreeSet::new();
    let mut endpoints = endpoints.into_iter().collect::<Vec<_>>();
    for endpoint in &endpoints {
        if !endpoint_ids.insert(endpoint.id.clone()) {
            return Err(FederationAnalysisError::DuplicateEndpointId {
                endpoint: endpoint.id.clone(),
            });
        }
        for member in [&endpoint.source, &endpoint.target] {
            if members.binary_search(member).is_err() {
                return Err(FederationAnalysisError::UndeclaredEndpointMember {
                    endpoint: endpoint.id.clone(),
                    federate: member.clone(),
                });
            }
        }
    }
    endpoints.sort_by(|left, right| {
        (&left.source, &left.target, &left.id, left.delay).cmp(&(
            &right.source,
            &right.target,
            &right.id,
            right.delay,
        ))
    });

    let mut graph = StableDiGraph::<FederateId, u128>::new();
    let mut nodes = BTreeMap::<FederateId, NodeIndex>::new();
    for member in &members {
        nodes.insert(member.clone(), graph.add_node(member.clone()));
    }
    for endpoint in &endpoints {
        graph.add_edge(
            nodes[&endpoint.source],
            nodes[&endpoint.target],
            u128::from(endpoint.delay.as_nanos()),
        );
    }

    validate_zero_delay_cycles(&graph)?;

    let mut direct_incoming = members
        .iter()
        .cloned()
        .map(|member| (member, BTreeMap::new()))
        .collect::<BTreeMap<FederateId, BTreeMap<FederateId, FederationDelay>>>();
    for endpoint in &endpoints {
        let dependencies = direct_incoming
            .get_mut(&endpoint.target)
            .expect("validated target must be a declared member");
        dependencies
            .entry(endpoint.source.clone())
            .and_modify(|delay| *delay = (*delay).min(endpoint.delay))
            .or_insert(endpoint.delay);
    }

    let mut transitive_incoming = members
        .iter()
        .cloned()
        .map(|member| (member, Vec::new()))
        .collect::<BTreeMap<_, _>>();
    let mut affected_downstream = members
        .iter()
        .cloned()
        .map(|member| (member, Vec::new()))
        .collect::<BTreeMap<_, _>>();
    for source in &members {
        for (target_node, delay) in minimum_nonempty_paths(&graph, nodes[source]) {
            let target = graph[target_node].clone();
            let delay = u64::try_from(delay).map_err(|_| {
                FederationAnalysisError::AccumulatedDelayOverflow {
                    from: source.clone(),
                    target: target.clone(),
                }
            })?;
            transitive_incoming
                .get_mut(&target)
                .expect("path target belongs to the graph")
                .push((source.clone(), FederationDelay::from_nanos(delay)));
            if source != &target {
                affected_downstream
                    .get_mut(source)
                    .expect("path source belongs to the graph")
                    .push(target);
            }
        }
    }

    Ok(AnalyzedFederationGraph {
        members,
        endpoints,
        direct_incoming: direct_incoming
            .into_iter()
            .map(|(member, dependencies)| (member, dependencies.into_iter().collect()))
            .collect(),
        transitive_incoming,
        affected_downstream,
    })
}

fn minimum_nonempty_paths(
    graph: &StableDiGraph<FederateId, u128>,
    source: NodeIndex,
) -> BTreeMap<NodeIndex, u128> {
    let mut distances = BTreeMap::new();
    let mut pending = BinaryHeap::new();

    for edge in graph.edges(source) {
        let target = edge.target();
        let delay = *edge.weight();
        if distances
            .get(&target)
            .is_none_or(|current| delay < *current)
        {
            distances.insert(target, delay);
            pending.push(Reverse((delay, target)));
        }
    }

    while let Some(Reverse((delay, node))) = pending.pop() {
        if distances.get(&node) != Some(&delay) {
            continue;
        }
        for edge in graph.edges(node) {
            let candidate = delay + *edge.weight();
            let target = edge.target();
            if distances
                .get(&target)
                .is_none_or(|current| candidate < *current)
            {
                distances.insert(target, candidate);
                pending.push(Reverse((candidate, target)));
            }
        }
    }

    distances
}

fn validate_zero_delay_cycles(
    graph: &StableDiGraph<FederateId, u128>,
) -> Result<(), FederationAnalysisError> {
    let mut zero_delay = StableDiGraph::<FederateId, ()>::new();
    let mut nodes = BTreeMap::new();
    for node in graph.node_indices() {
        nodes.insert(node, zero_delay.add_node(graph[node].clone()));
    }
    for edge in graph.edge_references().filter(|edge| *edge.weight() == 0) {
        zero_delay.add_edge(nodes[&edge.source()], nodes[&edge.target()], ());
    }

    let Err(cycle) = toposort(&zero_delay, None) else {
        return Ok(());
    };
    let mut cycles = kosaraju_scc(&zero_delay)
        .into_iter()
        .filter(|component| {
            component.len() > 1
                || component
                    .first()
                    .is_some_and(|node| zero_delay.find_edge(*node, *node).is_some())
        })
        .map(|component| {
            let mut members = component
                .into_iter()
                .map(|node| zero_delay[node].clone())
                .collect::<Vec<_>>();
            members.sort();
            members
        })
        .collect::<Vec<_>>();
    cycles.sort();

    Err(FederationAnalysisError::ZeroDelayCycle {
        federates: cycles
            .into_iter()
            .next()
            .unwrap_or_else(|| vec![zero_delay[cycle.node_id()].clone()]),
    })
}

#[cfg(test)]
mod tests {
    use super::{
        analyze_federation_graph, AnalyzedFederationGraph, FederationAnalysisError,
        FederationDelay, FederationEndpoint,
    };
    use crate::compiler::{BoundaryId, FederateId};

    fn federate(id: &str) -> FederateId {
        FederateId::new(id).unwrap()
    }

    fn endpoint(source: &str, target: &str, id: &str, delay_ns: u64) -> FederationEndpoint {
        FederationEndpoint::new(
            federate(source),
            federate(target),
            BoundaryId::new(id).unwrap(),
            FederationDelay::from_nanos(delay_ns),
        )
    }

    fn analyze(
        members: impl IntoIterator<Item = FederateId>,
        endpoints: impl IntoIterator<Item = FederationEndpoint>,
    ) -> Result<AnalyzedFederationGraph, FederationAnalysisError> {
        analyze_federation_graph(members, endpoints)
    }

    fn delay_for(
        dependencies: Option<&[(FederateId, FederationDelay)]>,
        source: &str,
    ) -> Option<u64> {
        dependencies?
            .iter()
            .find(|(candidate, _)| candidate == &federate(source))
            .map(|(_, delay)| delay.as_nanos())
    }

    #[test]
    fn canonicalizes_reordered_members_and_endpoints() {
        let members = [federate("c"), federate("a"), federate("b")];
        let endpoints = [
            endpoint("a", "b", "a-b", 3),
            endpoint("b", "c", "b-c", 4),
            endpoint("a", "c", "a-c", 9),
        ];
        let mut reversed_members = members.clone();
        reversed_members.reverse();
        let mut reversed_endpoints = endpoints.clone();
        reversed_endpoints.reverse();

        let graph = analyze(members, endpoints).unwrap();
        assert_eq!(
            graph,
            analyze(reversed_members, reversed_endpoints).unwrap()
        );
        assert_eq!(
            graph
                .members()
                .iter()
                .map(FederateId::as_str)
                .collect::<Vec<_>>(),
            vec!["a", "b", "c"]
        );
        assert_eq!(
            graph
                .endpoints()
                .iter()
                .map(|endpoint| endpoint.id().to_canonical_string())
                .collect::<Vec<_>>(),
            vec!["a-b", "a-c", "b-c"]
        );
    }

    #[test]
    fn preserves_parallel_endpoints_but_collapses_direct_dependencies() {
        let graph = analyze(
            [federate("a"), federate("b")],
            [endpoint("a", "b", "slow", 5), endpoint("a", "b", "fast", 2)],
        )
        .unwrap();

        assert_eq!(graph.endpoints().len(), 2);
        assert_eq!(
            delay_for(graph.direct_incoming(&federate("b")), "a"),
            Some(2)
        );
        assert_eq!(
            graph
                .endpoints()
                .iter()
                .map(|endpoint| endpoint.id().to_canonical_string())
                .collect::<Vec<_>>(),
            vec!["fast", "slow"]
        );
    }

    #[test]
    fn finds_cheaper_indirect_transitive_path_than_direct_endpoint() {
        let graph = analyze(
            [federate("a"), federate("b"), federate("c")],
            [
                endpoint("a", "b", "direct", 5),
                endpoint("a", "c", "via-c", 1),
                endpoint("c", "b", "to-b", 1),
            ],
        )
        .unwrap();

        assert_eq!(
            delay_for(graph.direct_incoming(&federate("b")), "a"),
            Some(5)
        );
        assert_eq!(
            delay_for(graph.transitive_incoming(&federate("b")), "a"),
            Some(2)
        );
    }

    #[test]
    fn rejects_zero_delay_cycles() {
        let error = analyze(
            [federate("b"), federate("a")],
            [endpoint("a", "b", "a-b", 0), endpoint("b", "a", "b-a", 0)],
        )
        .unwrap_err();

        assert!(matches!(
            error,
            FederationAnalysisError::ZeroDelayCycle { federates }
                if federates.iter().map(FederateId::as_str).collect::<Vec<_>>() == ["a", "b"]
        ));
    }

    #[test]
    fn accepts_positive_delay_cycles_and_includes_minimum_nonempty_self_path() {
        let graph = analyze(
            [federate("a"), federate("b")],
            [endpoint("a", "b", "a-b", 5), endpoint("b", "a", "b-a", 7)],
        )
        .unwrap();

        assert_eq!(
            delay_for(graph.transitive_incoming(&federate("a")), "a"),
            Some(12)
        );
    }

    #[test]
    fn retains_disconnected_members_without_dependencies() {
        let graph = analyze(
            [
                federate("isolated-b"),
                federate("connected"),
                federate("isolated-a"),
            ],
            [],
        )
        .unwrap();

        for member in graph.members() {
            assert_eq!(graph.direct_incoming(member), Some(&[][..]));
            assert_eq!(graph.transitive_incoming(member), Some(&[][..]));
            assert_eq!(graph.affected_downstream(member), Some(&[][..]));
        }
    }

    #[test]
    fn rejects_duplicate_federate_ids() {
        let error = analyze([federate("a"), federate("a")], []).unwrap_err();

        assert!(matches!(
            error,
            FederationAnalysisError::DuplicateFederateId { federate }
                if federate.as_str() == "a"
        ));
    }

    #[test]
    fn rejects_duplicate_endpoint_ids() {
        let error = analyze(
            [federate("a"), federate("b"), federate("c")],
            [
                endpoint("a", "b", "duplicate", 1),
                endpoint("a", "c", "duplicate", 2),
            ],
        )
        .unwrap_err();

        assert!(matches!(
            error,
            FederationAnalysisError::DuplicateEndpointId { endpoint }
                if endpoint.to_canonical_string() == "duplicate"
        ));
    }

    #[test]
    fn rejects_endpoints_with_undeclared_members() {
        let error =
            analyze([federate("a")], [endpoint("a", "missing", "a-missing", 1)]).unwrap_err();

        assert!(matches!(
            error,
            FederationAnalysisError::UndeclaredEndpointMember { endpoint, federate }
                if endpoint.to_canonical_string() == "a-missing" && federate.as_str() == "missing"
        ));
    }

    #[test]
    fn records_affected_downstream_sets() {
        let graph = analyze(
            [
                federate("isolated"),
                federate("b"),
                federate("a"),
                federate("c"),
            ],
            [
                endpoint("a", "b", "a-b", 2),
                endpoint("a", "c", "a-c", 1),
                endpoint("c", "b", "c-b", 1),
            ],
        )
        .unwrap();

        assert_eq!(
            graph
                .affected_downstream(&federate("a"))
                .unwrap()
                .iter()
                .map(FederateId::as_str)
                .collect::<Vec<_>>(),
            vec!["b", "c"]
        );
        assert_eq!(
            graph.affected_downstream(&federate("isolated")),
            Some(&[][..])
        );
    }

    #[test]
    fn rejects_accumulated_delay_overflow() {
        let error = analyze(
            [federate("a"), federate("b"), federate("c")],
            [
                endpoint("a", "b", "a-b", u64::MAX),
                endpoint("b", "c", "b-c", 1),
            ],
        )
        .unwrap_err();

        assert!(matches!(
            error,
            FederationAnalysisError::AccumulatedDelayOverflow { from, target }
                if from.as_str() == "a" && target.as_str() == "c"
        ));
    }
}
