//! External route binding and owned-slice entry point for compiled Federate execution.
use super::*;
use crate::{
    image::{FederateImage, RouteImage, RouteIndex},
    InboundBoundaryAdapter, OutboundBoundarySink, PayloadDecoder, PayloadEncoder,
};
use std::sync::Arc;

/// Installs a type-checked external adapter after all initializer-free validation.
type InstallRoute<'binding> = dyn for<'image> FnOnce(
        &mut OwnedStorage<'image>,
        RouteIndex,
        &'image RouteImage<'image>,
    ) -> Option<InboundBoundaryAdapter>
    + Send
    + 'binding;

/// One caller-supplied codec binding for a single external boundary half.
pub(super) struct ExternalRoute<'binding> {
    /// Stable identity shared with the missing peer's compiled route half.
    pub(super) boundary: BoundaryId<'binding>,
    /// Direction that this binding is allowed to serve.
    direction: RouteDirection,
    /// Concrete type checked against the compiled local port's direct binding.
    payload_type: (TypeId, &'static str),
    /// Deferred typed installation, invoked once after complete preflight.
    install: Box<InstallRoute<'binding>>,
}

impl<'binding> FederateBindings<'binding> {
    /// Binds an external outbound half to a codec and a nonblocking transport sink.
    ///
    /// The executor applies the compiled logical delay once, before sending. The sink
    /// must preserve ordering with backend publications and completions for this Federate.
    pub fn bind_outbound_route<T: ReactorData>(
        mut self,
        boundary: BoundaryId<'binding>,
        _payload: PayloadType<T>,
        encoder: impl PayloadEncoder<T>,
        sink: Arc<dyn OutboundBoundarySink>,
    ) -> Self {
        self.external_routes.push(ExternalRoute {
            boundary,
            direction: RouteDirection::Outbound,
            payload_type: (TypeId::of::<T>(), std::any::type_name::<T>()),
            install: Box::new(move |storage, local_route, route| {
                storage.bind_external_outbound(
                    route.local_port(),
                    Box::new(EncodedOutbound {
                        local_route,
                        route,
                        encoder,
                        payload_type: std::marker::PhantomData::<fn() -> T>,
                        sink,
                    }),
                );
                None
            }),
        });
        self
    }

    /// Binds an external inbound half to its typed decoder and compiled destination port.
    ///
    /// The backend receives the installed adapter before schedulers start and must admit
    /// preceding payloads before returning a grant. Received tags already include route delay.
    pub fn bind_inbound_route<T: ReactorData>(
        mut self,
        boundary: BoundaryId<'binding>,
        _payload: PayloadType<T>,
        decoder: impl PayloadDecoder<T>,
    ) -> Self {
        self.external_routes.push(ExternalRoute {
            boundary,
            direction: RouteDirection::Inbound,
            payload_type: (TypeId::of::<T>(), std::any::type_name::<T>()),
            install: Box::new(move |storage, _, route| {
                Some(InboundBoundaryAdapter::for_port(
                    storage.scheduler_event_tx(),
                    route.local_port(),
                    decoder,
                ))
            }),
        });
        self
    }
}

/// Encodes an owned port value and submits the already delay-adjusted logical tag.
struct EncodedOutbound<'image, T: ReactorData, C> {
    /// Typed key within the owning Enclave's route table, not an RTI route key.
    local_route: RouteIndex,
    /// Compiled outbound half defining source port, identity, and delay.
    route: &'image RouteImage<'image>,
    /// Typed encoder selected by direct generated bindings.
    encoder: C,
    /// Concrete port value type accepted by the encoder.
    payload_type: std::marker::PhantomData<fn() -> T>,
    /// Ordered nonblocking transport submission sink.
    sink: Arc<dyn OutboundBoundarySink>,
}

impl<T: ReactorData, C: PayloadEncoder<T>> crate::storage::owned::OutboundRoute
    for EncodedOutbound<'_, T, C>
{
    fn emit(&mut self, source: &dyn crate::BasePort, tag: Tag) -> Result<(), OwnedStorageError> {
        let boundary = self.route.boundary().as_str();
        let typed = source.downcast_ref::<crate::Port<T>>().ok_or_else(|| {
            OwnedStorageError::OutboundRoutePayloadTypeMismatch {
                boundary: boundary.to_owned(),
                port: source.get_key(),
                expected: std::any::type_name::<T>(),
                found: source.type_name(),
            }
        })?;
        let Some(value) = typed.get().as_ref() else {
            return Ok(());
        };
        let delay_nanos = self.route.delay_nanos();
        let target = if delay_nanos == 0 {
            tag
        } else {
            tag.checked_delay(crate::Duration::nanoseconds(
                i64::try_from(delay_nanos).expect("external route delay passed preflight"),
            ))
            .ok_or_else(|| OwnedStorageError::OutboundRouteTagOverflow {
                boundary: boundary.to_owned(),
                tag,
                delay_nanos,
            })?
        };
        let payload = self.encoder.encode(value).map_err(|source| {
            OwnedStorageError::ExternalRouteEncoding {
                boundary: boundary.to_owned(),
                source: crate::PayloadCodecError::new(source.to_string()),
            }
        })?;
        tracing::debug!(
            target: "boomerang::coordination",
            event = "coordination.codec.encoded",
            local_route = self.local_route.as_u32(),
            port = source.get_key().as_u32(),
            source_tag_kind = tag.kind_str(),
            source_tag_offset_ns = tag.offset().whole_nanoseconds(),
            source_tag_microstep = tag.microstep(),
            tag_kind = target.kind_str(),
            tag_offset_ns = target.offset().whole_nanoseconds(),
            tag_microstep = target.microstep(),
        );
        self.sink
            .send(crate::TaggedPayload {
                tag: target,
                payload,
            })
            .map_err(|source| OwnedStorageError::ExternalRouteSubmission {
                boundary: boundary.to_owned(),
                source,
            })
    }
}

/// Executes a generated Federate's owned Enclave slice through an injected backend.
///
/// `images` must contain checked views for exactly `image.enclaves()` in canonical order. Canonical
/// keys are retained; peer scheduler images and payload bindings are unnecessary. This validates
/// the owned slice and its bindings, not the absent global federation. The deployment compiler
/// and backend handshake must establish that the peer slices share the same coordination image.
///
/// Every route must pair locally or have exactly one matching external binding. This baseline
/// supports logical external routes only. Preflight completes before user initialization or
/// `connect`; the latter installs a backend from the prepared inbound boundary adapters. Callers own
/// transport readiness bounds. Backend failure aborts and joins the same scheduler threads used
/// by local execution. Idle termination additionally requires backend confirmation.
pub fn execute_owned_federate_with_backend<'image, B: FederateCoordinationBackend>(
    federate: FederateIndex,
    image: &FederateImage<'image>,
    images: &[&EnclaveImageView<'image>],
    mut bindings: FederateBindings<'_>,
    config: Config,
    connect: impl FnOnce(
        BTreeMap<BoundaryId<'image>, InboundBoundaryAdapter>,
    ) -> Result<B, crate::FederateCoordinationError>,
) -> Result<FederateExecution, ExecuteOwnedFederateError> {
    tracing::debug!(target: "boomerang::runtime",
        event = "runtime.preflight.started", owner = "federate", federate = federate.as_u32());
    let preflight = || {
        let images = prepare_images(image, images)?;
        preflight_enclave_bindings(federate, &images, &bindings)?;
        let (endpoints, external) = resolve_routes(federate, &images, &bindings)?;
        preflight_local_bindings(federate, &images, &endpoints, &bindings)?;
        Ok((images, endpoints, external))
    };
    let (images, endpoints, external) = match preflight() {
        Ok(prepared) => prepared,
        Err(error) => {
            tracing::warn!(target: "boomerang::runtime", event = "runtime.preflight.rejected",
                owner = "federate", federate = federate.as_u32(), reason = preflight_reason(&error));
            return Err(error);
        }
    };
    let span = federate_span(federate, image, "distributed");
    let _span = span.enter();
    tracing::debug!(target: "boomerang::runtime",
        event = "runtime.preflight.completed", owner = "federate", federate = federate.as_u32());
    let adapters = std::mem::take(&mut bindings.external_routes);
    let lifecycle = if config.keep_alive {
        LifecyclePolicy::KeepAlive
    } else {
        LifecyclePolicy::CoordinatedTermination
    };
    execute_prepared_federate(
        PreparedFederate { images, endpoints },
        bindings,
        config,
        lifecycle,
        move |storages| {
            let mut inbound = BTreeMap::new();
            for (adapter, (enclave, index, route)) in adapters.into_iter().zip(external) {
                let storage = &mut storages
                    .iter_mut()
                    .find(|(key, _)| *key == enclave)
                    .expect("validated external route belongs to owned storage")
                    .1;
                if let Some(adapter) = (adapter.install)(storage, index, route) {
                    inbound.insert(route.boundary(), adapter);
                }
            }
            connect(inbound)
                .map_err(|source| ExecuteOwnedFederateError::FederateCoordination { source })
        },
        |_| false,
    )
}

/// Executes a generated Federate's checked Enclave views with local coordination.
///
/// The views must match `image.enclaves()` in canonical order. Every route must pair within
/// this slice. Binding, payload, storage, and timing checks still precede user initialization.
/// Exogenous event sources require [`Config::keep_alive`] and explicit shutdown.
pub fn execute_owned_federate_slice<'image>(
    federate: FederateIndex,
    image: &FederateImage<'image>,
    images: &[&EnclaveImageView<'image>],
    bindings: FederateBindings<'_>,
    config: Config,
) -> Result<FederateExecution, ExecuteOwnedFederateError> {
    tracing::debug!(target: "boomerang::runtime",
        event = "runtime.preflight.started", owner = "federate", federate = federate.as_u32());
    let preflight = || {
        let images = prepare_images(image, images)?;
        preflight_enclave_bindings(federate, &images, &bindings)?;
        if let Some(route) = bindings.external_routes.first() {
            return Err(ExecuteOwnedFederateError::UnexpectedRouteBinding {
                boundary: route.boundary.as_str().to_owned(),
                federate,
            });
        }
        let (endpoints, _) = resolve_routes(federate, &images, &bindings)?;
        preflight_local_bindings(federate, &images, &endpoints, &bindings)?;
        Ok(PreparedFederate { images, endpoints })
    };
    let prepared = match preflight() {
        Ok(prepared) => prepared,
        Err(error) => {
            tracing::warn!(target: "boomerang::runtime", event = "runtime.preflight.rejected",
                owner = "federate", federate = federate.as_u32(), reason = preflight_reason(&error));
            return Err(error);
        }
    };
    let span = federate_span(federate, image, "local");
    let _span = span.enter();
    tracing::debug!(target: "boomerang::runtime",
        event = "runtime.preflight.completed", owner = "federate", federate = federate.as_u32());
    let lifecycle = if config.keep_alive {
        LifecyclePolicy::KeepAlive
    } else {
        LifecyclePolicy::TerminateWhenIdle
    };
    execute_prepared_federate(
        prepared,
        bindings,
        config,
        lifecycle,
        |_| Ok(LocalFederateCoordinationBackend::default()),
        |_| false,
    )
}

/// Validates owned layout coordinates before materializing the sparse canonical lookup.
fn prepare_images<'image>(
    image: &FederateImage<'image>,
    images: &[&EnclaveImageView<'image>],
) -> Result<TinySecondaryMap<EnclaveIndex, EnclaveImageView<'image>>, ExecuteOwnedFederateError> {
    let fail = |message: &str| ExecuteOwnedFederateError::ImageValidation {
        message: message.into(),
    };
    let span = image.enclaves();
    let end = span
        .checked_end()
        .ok_or_else(|| fail("Federate Enclave span overflows"))?;
    if images.len() != span.len() || end > <EnclaveIndex as tinymap::Key>::MAX_LEN {
        return Err(fail(
            "Federate Enclave slice does not match its canonical span",
        ));
    }
    for id in [
        image.id().as_str(),
        image.target().as_str(),
        image.runtime().as_str(),
    ] {
        if id.is_empty() || id.trim() != id || id.chars().any(char::is_control) {
            return Err(fail("invalid Federate identity, target or runtime"));
        }
    }
    if images
        .windows(2)
        .any(|pair| pair[0].enclave_id() >= pair[1].enclave_id())
    {
        return Err(fail(
            "Federate Enclave identities must be unique and sorted",
        ));
    }
    Ok((span.start()..end)
        .zip(images)
        .map(|(key, image)| (EnclaveIndex::from(key), image.reborrow()))
        .collect())
}

/// Local route pairs plus external coordinates in caller binding order.
type ResolvedRoutes<'image> = (
    Vec<ResolvedLocalRoute<'image>>,
    Vec<(EnclaveIndex, RouteIndex, &'image RouteImage<'image>)>,
);

/// Matches compiled halves by stable boundary identity without inferring peer key domains.
fn resolve_routes<'image>(
    federate: FederateIndex,
    images: &TinySecondaryMap<EnclaveIndex, EnclaveImageView<'image>>,
    bindings: &FederateBindings<'_>,
) -> Result<ResolvedRoutes<'image>, ExecuteOwnedFederateError> {
    let mut halves = BTreeMap::<_, (Option<_>, Option<_>)>::new();
    let invalid = |message: String| ExecuteOwnedFederateError::ImageValidation { message };
    for (enclave, image) in images.iter() {
        for (index, route) in image.routes().iter() {
            let pair = halves.entry(route.boundary()).or_default();
            let half = match route.direction() {
                RouteDirection::Outbound => &mut pair.0,
                RouteDirection::Inbound => &mut pair.1,
            };
            if half.replace((enclave, index, route)).is_some() {
                return Err(invalid(format!(
                    "duplicate {:?} half for {}",
                    route.direction(),
                    route.boundary().as_str()
                )));
            }
        }
    }
    let mut local = Vec::new();
    let mut external = BTreeMap::new();
    for (boundary, (outbound, inbound)) in halves {
        match (outbound, inbound) {
            (Some((source, index, out)), Some((destination, _, input))) => {
                if out.timing_domain() != input.timing_domain()
                    || out.delay_nanos() != input.delay_nanos()
                {
                    return Err(invalid(format!(
                        "local route pair disagrees for {}",
                        boundary.as_str()
                    )));
                }
                local.push(ResolvedLocalRoute {
                    key: LocalRouteKey {
                        source,
                        outbound: index,
                    },
                    boundary,
                    source,
                    source_port: out.local_port(),
                    destination,
                    destination_port: input.local_port(),
                    timing_domain: out.timing_domain(),
                    delay_nanos: out.delay_nanos(),
                });
            }
            (Some((enclave, index, route)), None) | (None, Some((enclave, index, route))) => {
                external.insert(boundary, (enclave, index, route));
            }
            (None, None) => unreachable!("each boundary has at least one compiled half"),
        }
    }
    let mut resolved = Vec::new();
    let mut bound = std::collections::BTreeSet::new();
    for binding in &bindings.external_routes {
        if !bound.insert(binding.boundary) {
            return Err(ExecuteOwnedFederateError::DuplicateRouteBinding {
                boundary: binding.boundary.as_str().to_owned(),
            });
        }
        let (enclave, index, route) = external.remove(&binding.boundary).ok_or_else(|| {
            ExecuteOwnedFederateError::UnexpectedRouteBinding {
                boundary: binding.boundary.as_str().to_owned(),
                federate,
            }
        })?;
        if route.direction() != binding.direction || route.timing_domain() != TimingDomain::Logical
        {
            return Err(invalid(format!(
                "external route {} has incompatible direction or timing",
                binding.boundary.as_str()
            )));
        }
        if route.delay_nanos() > i64::MAX.unsigned_abs() {
            return Err(ExecuteOwnedFederateError::RouteDelayOutOfRange {
                boundary: binding.boundary.as_str().to_owned(),
                delay_nanos: route.delay_nanos(),
            });
        }
        let slot = images[enclave].ports()[route.local_port()].binding();
        let (found_id, found) = bindings.enclaves[enclave]
            .port_payload_type(slot)
            .expect("port bindings passed preflight");
        let (expected_id, expected) = binding.payload_type;
        if found_id != expected_id {
            return Err(ExecuteOwnedFederateError::RoutePayloadTypeMismatch {
                boundary: binding.boundary.as_str().to_owned(),
                direction: binding.direction,
                enclave,
                port: route.local_port(),
                expected,
                found,
            });
        }
        resolved.push((enclave, index, route));
    }
    if let Some(boundary) = external.keys().next() {
        return Err(ExecuteOwnedFederateError::MissingRouteBinding {
            boundary: boundary.as_str().to_owned(),
        });
    }
    Ok((local, resolved))
}
