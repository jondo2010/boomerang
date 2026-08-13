use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet, BinaryHeap},
};

use boomerang_federated::{EndpointId, FederateId, WireDelay};
use petgraph::algo::{kosaraju_scc, toposort};
use petgraph::stable_graph::{NodeIndex, StableDiGraph};
use petgraph::visit::{EdgeRef, IntoEdgeReferences};

use crate::AssemblyError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FederationEndpoint {
    pub(crate) source: FederateId,
    pub(crate) target: FederateId,
    pub(crate) endpoint: EndpointId,
    pub(crate) delay: WireDelay,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AnalyzedFederationGraph {
    pub(crate) federates: Vec<FederateId>,
    pub(crate) endpoints: Vec<FederationEndpoint>,
    pub(crate) transitive_incoming: BTreeMap<FederateId, Vec<(FederateId, WireDelay)>>,
    pub(crate) affected_downstream: BTreeMap<FederateId, Vec<FederateId>>,
}

impl AnalyzedFederationGraph {
    /// Project completed analysis into the federated runtime's immutable graph handoff.
    pub(crate) fn to_rti_graph(&self) -> boomerang_federated::RtiGraph {
        let federates = self
            .federates
            .iter()
            .map(|id| boomerang_federated::rti::RtiFederateParts {
                id: id.clone(),
                transitive_incoming: self.transitive_incoming[id].clone(),
                affected_downstream: self.affected_downstream[id].clone(),
            })
            .collect();
        let endpoints = self
            .endpoints
            .iter()
            .map(|endpoint| boomerang_federated::rti::RtiEndpointParts {
                id: endpoint.endpoint.clone(),
                source: endpoint.source.clone(),
                target: endpoint.target.clone(),
                delay: endpoint.delay,
            })
            .collect();
        boomerang_federated::RtiGraph::from_lowered(boomerang_federated::rti::RtiGraphParts {
            federates,
            endpoints,
        })
    }
}

pub(crate) fn analyze_federation_graph(
    federates: impl IntoIterator<Item = FederateId>,
    endpoints: impl IntoIterator<Item = FederationEndpoint>,
) -> Result<AnalyzedFederationGraph, AssemblyError> {
    let mut members = BTreeSet::new();
    for federate in federates {
        if !members.insert(federate.clone()) {
            return Err(AssemblyError::DuplicateFederateId {
                federate_id: federate.to_string(),
            });
        }
    }
    let federates = members.into_iter().collect::<Vec<_>>();

    let mut endpoint_ids = BTreeSet::new();
    let mut endpoints = endpoints.into_iter().collect::<Vec<_>>();
    for edge in &endpoints {
        if !endpoint_ids.insert(edge.endpoint.clone()) {
            return Err(AssemblyError::DuplicateFederatedEndpoint {
                endpoint: edge.endpoint.to_string(),
            });
        }
        for federate in [&edge.source, &edge.target] {
            if federates.binary_search(federate).is_err() {
                return Err(AssemblyError::FederationBridgeError {
                    what: format!(
                        "federated endpoint '{}' references undeclared federate '{}'",
                        edge.endpoint, federate
                    ),
                });
            }
        }
    }
    endpoints.sort_by(|left, right| {
        (
            &left.source,
            &left.target,
            &left.endpoint,
            left.delay.as_nanos(),
        )
            .cmp(&(
                &right.source,
                &right.target,
                &right.endpoint,
                right.delay.as_nanos(),
            ))
    });

    let mut graph = StableDiGraph::<FederateId, u128>::new();
    let mut nodes = BTreeMap::<FederateId, NodeIndex>::new();
    for federate in &federates {
        nodes.insert(federate.clone(), graph.add_node(federate.clone()));
    }
    for edge in &endpoints {
        graph.add_edge(
            nodes[&edge.source],
            nodes[&edge.target],
            u128::from(edge.delay.as_nanos()),
        );
    }

    validate_zero_delay_cycles(&graph)?;

    let mut minimum_delays = BTreeMap::<(FederateId, FederateId), u128>::new();
    for source in &federates {
        let source_node = nodes[source];
        let distances = minimum_nonempty_paths(&graph, source_node);
        for target in &federates {
            let target_node = nodes[target];
            let Some(&delay) = distances.get(&target_node) else {
                continue;
            };
            minimum_delays.insert((source.clone(), target.clone()), delay);
        }
    }

    let mut transitive_incoming = federates
        .iter()
        .cloned()
        .map(|federate| (federate, Vec::new()))
        .collect::<BTreeMap<_, _>>();
    let mut affected_downstream = federates
        .iter()
        .cloned()
        .map(|federate| (federate, Vec::new()))
        .collect::<BTreeMap<_, _>>();
    for ((source, target), delay) in minimum_delays {
        let nanos =
            u64::try_from(delay).map_err(|_| AssemblyError::FederationPathDelayOverflow {
                source: source.to_string(),
                target: target.to_string(),
            })?;
        transitive_incoming
            .get_mut(&target)
            .expect("targets are declared federation members")
            .push((source.clone(), WireDelay::from_nanos(nanos)));
        if source != target {
            affected_downstream
                .get_mut(&source)
                .expect("sources are declared federation members")
                .push(target);
        }
    }

    Ok(AnalyzedFederationGraph {
        federates,
        endpoints,
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

    while let Some(Reverse((distance, node))) = pending.pop() {
        if distances.get(&node) != Some(&distance) {
            continue;
        }
        for edge in graph.edges(node) {
            let Some(candidate) = distance.checked_add(*edge.weight()) else {
                continue;
            };
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
) -> Result<(), AssemblyError> {
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
            let mut ids = component
                .into_iter()
                .map(|node| zero_delay[node].to_string())
                .collect::<Vec<_>>();
            ids.sort();
            ids
        })
        .collect::<Vec<_>>();
    cycles.sort();

    Err(AssemblyError::FederationZeroDelayCycle {
        federates: cycles
            .into_iter()
            .next()
            .unwrap_or_else(|| vec![zero_delay[cycle.node_id()].to_string()]),
    })
}

#[cfg(test)]
mod tests {
    use super::{analyze_federation_graph, FederationEndpoint};
    use crate::AssemblyError;
    use boomerang_federated::{EndpointId, FederateId, WireDelay};

    fn fed(id: &str) -> FederateId {
        FederateId::new(id)
    }

    fn endpoint(source: &str, target: &str, id: &str, delay_ns: u64) -> FederationEndpoint {
        FederationEndpoint {
            source: fed(source),
            target: fed(target),
            endpoint: EndpointId::new(id),
            delay: WireDelay::from_nanos(delay_ns),
        }
    }

    #[test]
    fn finds_minimum_nonempty_paths_and_affected_sets_by_stable_id() {
        let graph = analyze_federation_graph(
            [fed("isolated"), fed("b"), fed("a"), fed("c")],
            [
                endpoint("a", "b", "a-b-direct", 5),
                endpoint("a", "c", "a-c", 1),
                endpoint("c", "b", "c-b", 1),
                endpoint("b", "a", "b-a", 10),
            ],
        )
        .unwrap();

        assert_eq!(
            graph.federates,
            vec![fed("a"), fed("b"), fed("c"), fed("isolated")]
        );
        assert_eq!(
            graph.transitive_incoming[&fed("b")]
                .iter()
                .find(|(source, _)| source == &fed("a"))
                .map(|(_, delay)| delay.as_nanos()),
            Some(2)
        );
        assert_eq!(
            graph.transitive_incoming[&fed("a")]
                .iter()
                .find(|(source, _)| source == &fed("a"))
                .map(|(_, delay)| delay.as_nanos()),
            Some(12)
        );
        assert!(graph.transitive_incoming[&fed("a")]
            .iter()
            .all(|(source, _)| source != &fed("isolated")));
        assert_eq!(
            graph.affected_downstream[&fed("a")],
            vec![fed("b"), fed("c")]
        );
    }

    #[test]
    fn rejects_zero_delay_cycle_with_stable_federate_ids() {
        let error = analyze_federation_graph(
            [fed("b"), fed("a")],
            [endpoint("a", "b", "a-b", 0), endpoint("b", "a", "b-a", 0)],
        )
        .unwrap_err();

        assert!(matches!(
            error,
            AssemblyError::FederationZeroDelayCycle { federates }
                if federates == vec!["a".to_owned(), "b".to_owned()]
        ));
    }

    #[test]
    fn preserves_parallel_endpoints_between_the_same_federates() {
        let graph = analyze_federation_graph(
            [fed("a"), fed("b")],
            [
                endpoint("a", "b", "route-2", 2),
                endpoint("a", "b", "route-1", 1),
            ],
        )
        .unwrap();

        assert_eq!(graph.endpoints.len(), 2);
        assert_eq!(
            graph
                .endpoints
                .iter()
                .map(|edge| edge.endpoint.as_str())
                .collect::<Vec<_>>(),
            vec!["route-1", "route-2"]
        );
    }

    #[test]
    fn results_are_deterministic_under_input_reordering() {
        let members = [fed("c"), fed("a"), fed("b")];
        let edges = [
            endpoint("a", "b", "a-b", 3),
            endpoint("b", "c", "b-c", 4),
            endpoint("a", "c", "a-c", 9),
        ];
        let mut reversed_members = members.clone();
        reversed_members.reverse();
        let mut reversed_edges = edges.clone();
        reversed_edges.reverse();

        assert_eq!(
            analyze_federation_graph(members, edges).unwrap(),
            analyze_federation_graph(reversed_members, reversed_edges).unwrap()
        );
    }

    #[test]
    fn rejects_duplicate_endpoint_ids() {
        let error = analyze_federation_graph(
            [fed("a"), fed("b"), fed("c")],
            [
                endpoint("a", "b", "duplicate", 1),
                endpoint("a", "c", "duplicate", 2),
            ],
        )
        .unwrap_err();

        assert!(matches!(
            error,
            AssemblyError::DuplicateFederatedEndpoint { endpoint }
                if endpoint == "duplicate"
        ));
    }

    #[test]
    fn accepts_positive_delay_cycles() {
        let graph = analyze_federation_graph(
            [fed("a"), fed("b")],
            [endpoint("a", "b", "a-b", 5), endpoint("b", "a", "b-a", 7)],
        )
        .unwrap();

        assert_eq!(
            graph.transitive_incoming[&fed("a")]
                .iter()
                .find(|(source, _)| source == &fed("a"))
                .map(|(_, delay)| delay.as_nanos()),
            Some(12)
        );
    }

    #[test]
    fn retains_disconnected_members() {
        let graph =
            analyze_federation_graph([fed("isolated-b"), fed("connected"), fed("isolated-a")], [])
                .unwrap();

        assert_eq!(
            graph.federates,
            vec![fed("connected"), fed("isolated-a"), fed("isolated-b")]
        );
        for federate in &graph.federates {
            assert!(graph.transitive_incoming[federate].is_empty());
            assert!(graph.affected_downstream[federate].is_empty());
        }
    }

    #[test]
    fn rejects_accumulated_path_delay_greater_than_u64_max() {
        let error = analyze_federation_graph(
            [fed("a"), fed("b"), fed("c")],
            [
                endpoint("a", "b", "a-b", u64::MAX),
                endpoint("b", "c", "b-c", 1),
            ],
        )
        .unwrap_err();

        assert!(matches!(
            error,
            AssemblyError::FederationPathDelayOverflow { source, target }
                if source == "a" && target == "c"
        ));
    }

    #[test]
    fn rejects_duplicate_federate_ids() {
        let error = analyze_federation_graph([fed("a"), fed("a")], []).unwrap_err();

        assert!(matches!(
            error,
            AssemblyError::DuplicateFederateId { federate_id } if federate_id == "a"
        ));
    }
}
