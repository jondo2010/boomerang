use std::collections::BTreeMap;

use tinymap::TinyMap;

use super::index::{
    EndpointKey, FederateKey, IncomingDependency, IncomingPath, RtiEndpoint, RtiFederate,
};
use crate::protocol::{EndpointId, FederateId, WireDelay};

/// Immutable RTI graph indexed by dense runtime-local identities.
#[derive(Debug)]
pub struct RtiGraph {
    federates: TinyMap<FederateKey, RtiFederate>,
    federate_keys: BTreeMap<FederateId, FederateKey>,
    endpoints: TinyMap<EndpointKey, RtiEndpoint>,
    endpoint_keys: BTreeMap<EndpointId, EndpointKey>,
}

/// Final stable-identity records produced by federation lowering.
#[doc(hidden)]
pub struct RtiGraphParts {
    pub federates: Vec<RtiFederateParts>,
    pub endpoints: Vec<RtiEndpointParts>,
}

/// Final graph-analysis results for one Federate.
#[doc(hidden)]
pub struct RtiFederateParts {
    pub id: FederateId,
    pub transitive_incoming: Vec<(FederateId, WireDelay)>,
    pub affected_downstream: Vec<FederateId>,
}

/// Final lowered route for one federated endpoint.
#[doc(hidden)]
pub struct RtiEndpointParts {
    pub id: EndpointId,
    pub source: FederateId,
    pub target: FederateId,
    pub delay: WireDelay,
}

impl RtiGraph {
    /// Intern final lowered graph records without repeating graph analysis.
    #[doc(hidden)]
    pub fn from_lowered(mut parts: RtiGraphParts) -> Self {
        parts
            .federates
            .sort_by(|left, right| left.id.cmp(&right.id));
        parts
            .endpoints
            .sort_by(|left, right| left.id.cmp(&right.id));

        let mut federates = TinyMap::with_capacity(parts.federates.len());
        let mut federate_keys = BTreeMap::new();
        for federate in &parts.federates {
            let id = federate.id.clone();
            assert!(
                !federate_keys.contains_key(&id),
                "lowered RTI graph contains duplicate Federate ID '{id}'"
            );
            let key = federates.insert(RtiFederate {
                id: id.clone(),
                incoming: Vec::new(),
                transitive_incoming: Vec::new(),
                affected_downstream: Vec::new(),
            });
            federate_keys.insert(id, key);
        }

        let mut endpoints = TinyMap::with_capacity(parts.endpoints.len());
        let mut endpoint_keys = BTreeMap::new();
        for endpoint in parts.endpoints {
            assert!(
                !endpoint_keys.contains_key(&endpoint.id),
                "lowered RTI graph contains duplicate endpoint ID '{}'",
                endpoint.id
            );
            let source = federate_keys
                .get(&endpoint.source)
                .copied()
                .unwrap_or_else(|| {
                    panic!(
                        "lowered endpoint '{}' references missing source Federate '{}'",
                        endpoint.id, endpoint.source
                    )
                });
            let target = federate_keys
                .get(&endpoint.target)
                .copied()
                .unwrap_or_else(|| {
                    panic!(
                        "lowered endpoint '{}' references missing target Federate '{}'",
                        endpoint.id, endpoint.target
                    )
                });
            let id = endpoint.id;
            let delay = endpoint.delay;
            let key = endpoints.insert(RtiEndpoint {
                id: id.clone(),
                source,
                target,
                delay,
            });
            endpoint_keys.insert(id, key);
            federates[target].incoming.push(IncomingDependency {
                source,
                endpoint: key,
                delay,
            });
        }

        for incoming in federates
            .values_mut()
            .map(|federate| &mut federate.incoming)
        {
            incoming.sort();
        }

        for federate in parts.federates {
            let key = federate_keys
                .get(&federate.id)
                .copied()
                .unwrap_or_else(|| panic!("lowered Federate '{}' was not interned", federate.id));
            let mut transitive_incoming = federate
                .transitive_incoming
                .into_iter()
                .map(|(source, delay)| IncomingPath {
                    source: federate_keys.get(&source).copied().unwrap_or_else(|| {
                        panic!(
                            "lowered Federate '{}' has transitive incoming path from missing Federate '{}'",
                            federate.id, source
                        )
                    }),
                    delay,
                })
                .collect::<Vec<_>>();
            transitive_incoming.sort();
            let duplicate_incoming = transitive_incoming.windows(2).find_map(|pair| {
                (pair[0].source == pair[1].source).then(|| federates[pair[0].source].id.clone())
            });
            assert!(
                duplicate_incoming.is_none(),
                "lowered Federate '{}' contains duplicate transitive incoming source '{}'",
                federate.id,
                duplicate_incoming.unwrap()
            );

            let mut affected_downstream = federate
                .affected_downstream
                .into_iter()
                .map(|target| {
                    federate_keys.get(&target).copied().unwrap_or_else(|| {
                        panic!(
                            "lowered Federate '{}' has affected downstream target missing Federate '{}'",
                            federate.id, target
                        )
                    })
                })
                .collect::<Vec<_>>();
            affected_downstream.sort();
            let duplicate_affected = affected_downstream
                .windows(2)
                .find_map(|pair| (pair[0] == pair[1]).then(|| federates[pair[0]].id.clone()));
            assert!(
                duplicate_affected.is_none(),
                "lowered Federate '{}' contains duplicate affected downstream target '{}'",
                federate.id,
                duplicate_affected.unwrap()
            );

            federates[key].transitive_incoming = transitive_incoming;
            federates[key].affected_downstream = affected_downstream;
        }

        Self {
            federates,
            federate_keys,
            endpoints,
            endpoint_keys,
        }
    }

    /// Stable Federate identities in deterministic order.
    pub fn federate_ids(&self) -> impl Iterator<Item = &FederateId> {
        self.federates.values().map(|federate| &federate.id)
    }

    /// Stable endpoint identities in deterministic order.
    pub fn endpoint_ids(&self) -> impl Iterator<Item = &EndpointId> {
        self.endpoints.values().map(|endpoint| &endpoint.id)
    }

    /// Return the logical delay of a stable endpoint identity.
    pub fn endpoint_delay(&self, id: &EndpointId) -> Option<WireDelay> {
        self.endpoint_key(id).map(|key| self.endpoints[key].delay)
    }

    /// Final lowered endpoints with their stable source and target identities.
    ///
    /// This is a read-only view of the already-owned RTI graph; callers do not need to reconstruct
    /// endpoint relationships from pre-lowering declarations.
    pub fn endpoint_routes(
        &self,
    ) -> impl Iterator<Item = (&EndpointId, &FederateId, &FederateId, WireDelay)> {
        self.endpoints.values().map(|endpoint| {
            (
                &endpoint.id,
                &self.federates[endpoint.source].id,
                &self.federates[endpoint.target].id,
                endpoint.delay,
            )
        })
    }

    pub(crate) fn federate_key(&self, id: &FederateId) -> Option<FederateKey> {
        self.federate_keys.get(id).copied()
    }

    pub(crate) fn federate_id(&self, key: FederateKey) -> &FederateId {
        &self.federates[key].id
    }

    pub(crate) fn federates(&self) -> impl Iterator<Item = (FederateKey, &RtiFederate)> + '_ {
        self.federates.iter()
    }

    pub(crate) fn endpoint_key(&self, id: &EndpointId) -> Option<EndpointKey> {
        self.endpoint_keys.get(id).copied()
    }

    pub(crate) fn endpoint_id(&self, key: EndpointKey) -> &EndpointId {
        &self.endpoints[key].id
    }

    pub(crate) fn endpoint(&self, key: EndpointKey) -> &RtiEndpoint {
        &self.endpoints[key]
    }

    #[cfg(test)]
    pub(crate) fn endpoints(&self) -> impl Iterator<Item = (EndpointKey, &RtiEndpoint)> + '_ {
        self.endpoints.iter()
    }

    pub(super) fn incoming(&self, target: FederateKey) -> &[IncomingDependency] {
        &self.federates[target].incoming
    }

    pub(super) fn transitive_incoming(&self, target: FederateKey) -> &[IncomingPath] {
        &self.federates[target].transitive_incoming
    }

    pub(super) fn affected_downstream(&self, source: FederateKey) -> &[FederateKey] {
        &self.federates[source].affected_downstream
    }

    #[cfg(test)]
    pub(crate) fn contains_route(
        &self,
        source: &FederateId,
        target: &FederateId,
        endpoint: &EndpointId,
    ) -> bool {
        let Some(source) = self.federate_key(source) else {
            return false;
        };
        let Some(target) = self.federate_key(target) else {
            return false;
        };
        let Some(endpoint) = self.endpoint_key(endpoint) else {
            return false;
        };
        let route = &self.endpoints[endpoint];
        route.source == source && route.target == target
    }
}

#[cfg(test)]
mod tests {
    use super::{RtiEndpointParts, RtiFederateParts, RtiGraph, RtiGraphParts};
    use crate::protocol::{EndpointId, FederateId, WireDelay};

    fn fed(id: &str) -> FederateId {
        FederateId::new(id)
    }

    fn endpoint(id: &str) -> EndpointId {
        EndpointId::new(id)
    }

    fn federate_parts(
        id: &str,
        transitive_incoming: Vec<(&str, u64)>,
        affected_downstream: Vec<&str>,
    ) -> RtiFederateParts {
        RtiFederateParts {
            id: fed(id),
            transitive_incoming: transitive_incoming
                .into_iter()
                .map(|(source, delay)| (fed(source), WireDelay::from_nanos(delay)))
                .collect(),
            affected_downstream: affected_downstream.into_iter().map(fed).collect(),
        }
    }

    fn endpoint_parts(source: &str, target: &str, id: &str, delay: u64) -> RtiEndpointParts {
        RtiEndpointParts {
            id: endpoint(id),
            source: fed(source),
            target: fed(target),
            delay: WireDelay::from_nanos(delay),
        }
    }

    fn graph() -> RtiGraph {
        RtiGraph::from_lowered(RtiGraphParts {
            federates: vec![
                federate_parts("c", vec![("b", 2), ("a", 1)], vec![]),
                federate_parts("a", vec![], vec!["c", "b"]),
                federate_parts("b", vec![("a", 0)], vec!["c"]),
            ],
            endpoints: vec![
                endpoint_parts("b", "c", "b-c", 2),
                endpoint_parts("a", "c", "a-c", 1),
                endpoint_parts("a", "b", "a-b", 0),
            ],
        })
    }

    #[test]
    fn interns_stable_federate_ids_in_lexical_dense_order() {
        let graph = graph();
        let a = graph.federate_key(&fed("a")).unwrap();
        let b = graph.federate_key(&fed("b")).unwrap();
        let c = graph.federate_key(&fed("c")).unwrap();

        assert_eq!(
            graph.federates().map(|(key, _)| key).collect::<Vec<_>>(),
            vec![a, b, c]
        );
        assert_eq!(graph.federate_id(a), &fed("a"));
        assert_eq!(graph.federate_id(b), &fed("b"));
        assert_eq!(graph.federate_id(c), &fed("c"));
    }

    #[test]
    fn mechanically_translates_and_sorts_final_federate_sets() {
        let graph = graph();
        let a = graph.federate_key(&fed("a")).unwrap();
        let b = graph.federate_key(&fed("b")).unwrap();
        let c = graph.federate_key(&fed("c")).unwrap();

        assert_eq!(graph.affected_downstream(a), &[b, c]);
        assert_eq!(
            graph
                .transitive_incoming(c)
                .iter()
                .map(|path| (path.source, path.delay.as_nanos()))
                .collect::<Vec<_>>(),
            vec![(a, 1), (b, 2)]
        );
    }

    #[test]
    fn derives_sorted_direct_incoming_dependencies_from_endpoints() {
        let graph = graph();
        let a = graph.federate_key(&fed("a")).unwrap();
        let b = graph.federate_key(&fed("b")).unwrap();
        let c = graph.federate_key(&fed("c")).unwrap();
        let a_c = graph.endpoint_key(&endpoint("a-c")).unwrap();
        let b_c = graph.endpoint_key(&endpoint("b-c")).unwrap();

        assert_eq!(
            graph
                .incoming(c)
                .iter()
                .map(|dependency| (
                    dependency.source,
                    dependency.endpoint,
                    dependency.delay.as_nanos(),
                ))
                .collect::<Vec<_>>(),
            vec![(a, a_c, 1), (b, b_c, 2)]
        );
    }

    #[test]
    fn interns_endpoints_lexically_and_binds_exact_routes() {
        let graph = graph();
        let endpoint_ids = ["a-b", "a-c", "b-c"];

        assert_eq!(
            graph
                .endpoints()
                .map(|(_, endpoint)| endpoint.id.as_str())
                .collect::<Vec<_>>(),
            endpoint_ids
        );
        assert_eq!(
            graph
                .endpoint_routes()
                .map(|(id, source, target, delay)| (
                    id.as_str(),
                    source.as_str(),
                    target.as_str(),
                    delay.as_nanos(),
                ))
                .collect::<Vec<_>>(),
            vec![
                ("a-b", "a", "b", 0),
                ("a-c", "a", "c", 1),
                ("b-c", "b", "c", 2),
            ]
        );

        for id in endpoint_ids {
            let key = graph.endpoint_key(&endpoint(id)).unwrap();
            assert_eq!(graph.endpoint_id(key), &endpoint(id));
        }
        assert!(graph.contains_route(&fed("a"), &fed("c"), &endpoint("a-c")));
        assert!(!graph.contains_route(&fed("c"), &fed("a"), &endpoint("a-c")));
        assert!(!graph.contains_route(&fed("a"), &fed("b"), &endpoint("a-c")));
    }

    #[test]
    #[should_panic(expected = "lowered RTI graph contains duplicate Federate ID 'a'")]
    fn rejects_duplicate_top_level_federate_ids() {
        RtiGraph::from_lowered(RtiGraphParts {
            federates: vec![
                federate_parts("a", vec![], vec![]),
                federate_parts("a", vec![], vec![]),
            ],
            endpoints: vec![],
        });
    }

    #[test]
    #[should_panic(expected = "lowered RTI graph contains duplicate endpoint ID 'a-b'")]
    fn rejects_duplicate_top_level_endpoint_ids() {
        RtiGraph::from_lowered(RtiGraphParts {
            federates: vec![
                federate_parts("a", vec![], vec![]),
                federate_parts("b", vec![], vec![]),
            ],
            endpoints: vec![
                endpoint_parts("a", "b", "a-b", 1),
                endpoint_parts("a", "b", "a-b", 2),
            ],
        });
    }

    #[test]
    #[should_panic(
        expected = "lowered endpoint 'missing-a-b' references missing source Federate 'missing'"
    )]
    fn rejects_endpoint_with_missing_source_federate() {
        RtiGraph::from_lowered(RtiGraphParts {
            federates: vec![federate_parts("b", vec![], vec![])],
            endpoints: vec![endpoint_parts("missing", "b", "missing-a-b", 0)],
        });
    }

    #[test]
    #[should_panic(
        expected = "lowered Federate 'b' has transitive incoming path from missing Federate 'missing'"
    )]
    fn rejects_transitive_incoming_path_from_missing_federate() {
        RtiGraph::from_lowered(RtiGraphParts {
            federates: vec![federate_parts("b", vec![("missing", 1)], vec![])],
            endpoints: vec![],
        });
    }

    #[test]
    #[should_panic(
        expected = "lowered Federate 'a' has affected downstream target missing Federate 'missing'"
    )]
    fn rejects_affected_downstream_targeting_missing_federate() {
        RtiGraph::from_lowered(RtiGraphParts {
            federates: vec![federate_parts("a", vec![], vec!["missing"])],
            endpoints: vec![],
        });
    }

    #[test]
    #[should_panic(
        expected = "lowered Federate 'b' contains duplicate transitive incoming source 'a'"
    )]
    fn rejects_duplicate_transitive_incoming_sources_with_conflicting_delays() {
        RtiGraph::from_lowered(RtiGraphParts {
            federates: vec![
                federate_parts("a", vec![], vec![]),
                federate_parts("b", vec![("a", 1), ("a", 2)], vec![]),
            ],
            endpoints: vec![],
        });
    }

    #[test]
    #[should_panic(
        expected = "lowered Federate 'a' contains duplicate affected downstream target 'b'"
    )]
    fn rejects_duplicate_affected_downstream_targets() {
        RtiGraph::from_lowered(RtiGraphParts {
            federates: vec![
                federate_parts("a", vec![], vec!["b", "b"]),
                federate_parts("b", vec![], vec![]),
            ],
            endpoints: vec![],
        });
    }
}
