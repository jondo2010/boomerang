//! Owned compiled-image storage and immutable Federate slice projections.
use super::{
    BindingSlotId, BoundaryId, ComponentInstanceId, FederateId, ImplementationId,
    OwnedCoordinationProjection, RuntimeBackendId, StableEnclaveId, StablePath, TargetTriple,
};
use crate::descriptor::{ActionSlotId, PortSlotId, ReactionSlotId, ReactorSlotId};
use crate::runtime::image::{
    self as runtime_image, ActionImage, ActionIndex, BindingKind, BindingSlotIndex, EnclaveImage,
    EnclaveImageView, EnclaveIndex, FederateImage, FederateIndex, ImageValidationError,
    LevelReactionImage, LifecycleReactionImage, ModeImage, ModeIndex, PortImage, PortIndex,
    ReactionImage, ReactionIndex, ReactorImage, ReactorIndex, RequiredBindingImage, RouteImage,
    RouteIndex, ScopeImage, ScopeIndex, StorageBounds, TimerStartupImage,
};
use tinymap::{IndexSpan, TinyMap, TinyMapView};

/// Canonical required payload binding identities for one Enclave.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequiredBindings {
    /// Stable typed binding requirements.
    pub(crate) entries: Box<[RequiredBinding]>,
}

impl RequiredBindings {
    /// Iterates required bindings in canonical stable-identity order.
    pub fn iter(&self) -> impl Iterator<Item = &RequiredBinding> {
        self.entries.iter()
    }
}

/// One direct typed payload binding required by an Enclave image.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RequiredBinding {
    /// Initializes and owns the state object for one reactor.
    State {
        /// Logical component instance receiving this binding.
        component: ComponentInstanceId,
        /// Selected implementation exporting this binding's symbol.
        implementation: ImplementationId,
        /// Implementation-local descriptor reactor slot.
        reactor: ReactorSlotId,
    },
    /// Invokes one user reaction implementation.
    Reaction {
        /// Logical component instance receiving this binding.
        component: ComponentInstanceId,
        /// Selected implementation exporting this binding's symbol.
        implementation: ImplementationId,
        /// Implementation-local descriptor reaction slot.
        reaction: ReactionSlotId,
    },
    /// Constructs one concrete port payload type.
    Port {
        /// Logical component instance receiving this binding.
        component: ComponentInstanceId,
        /// Selected implementation exporting this binding's symbol.
        implementation: ImplementationId,
        /// Implementation-local descriptor port slot.
        port: PortSlotId,
    },
    /// Constructs one concrete standard-action payload type.
    Action {
        /// Logical component instance receiving this binding.
        component: ComponentInstanceId,
        /// Selected implementation exporting this binding's symbol.
        implementation: ImplementationId,
        /// Implementation-local descriptor action slot.
        action: ActionSlotId,
    },
}

impl RequiredBinding {
    /// Returns the runtime binding category.
    pub const fn kind(&self) -> BindingKind {
        match self {
            Self::State { .. } => BindingKind::StateInitializer,
            Self::Reaction { .. } => BindingKind::Reaction,
            Self::Port { .. } => BindingKind::Port,
            Self::Action { .. } => BindingKind::Action,
        }
    }

    /// Returns the implementation-local Rust symbol required for this binding.
    pub fn symbol(&self) -> String {
        match self {
            Self::State { reactor, .. } => {
                direct_binding_symbol(BindingKind::StateInitializer, reactor.path())
            }
            Self::Reaction { reaction, .. } => {
                direct_binding_symbol(BindingKind::Reaction, reaction.path())
            }
            Self::Port { port, .. } => direct_binding_symbol(BindingKind::Port, port.path()),
            Self::Action { action, .. } => {
                direct_binding_symbol(BindingKind::Action, action.path())
            }
        }
    }
}

/// Returns the canonical Rust symbol for one implementation-local descriptor slot.
pub fn direct_binding_symbol(kind: BindingKind, slot: &StablePath) -> String {
    use std::fmt::Write as _;

    let mut symbol = match kind {
        BindingKind::StateInitializer => String::from("state_"),
        BindingKind::Reaction => String::from("reaction_"),
        BindingKind::Port => String::from("port_"),
        BindingKind::Action => String::from("action_"),
    };
    for byte in slot.to_string().bytes() {
        if byte.is_ascii_alphanumeric() {
            symbol.push(char::from(byte));
        } else {
            write!(symbol, "_{byte:02x}").expect("writing into a String cannot fail");
        }
    }
    symbol
}

/// Heap-backed immutable scheduler image for one Enclave.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnedEnclaveImage {
    /// Stable Enclave identity.
    pub(crate) id: StableEnclaveId,
    /// Dense reactor rows.
    pub(crate) reactors: TinyMap<ReactorIndex, ReactorImage>,
    /// Dense action rows.
    pub(crate) actions: TinyMap<ActionIndex, ActionImage>,
    /// Dense port rows.
    pub(crate) ports: TinyMap<PortIndex, PortImage>,
    /// Dense reaction rows.
    pub(crate) reactions: TinyMap<ReactionIndex, ReactionImage>,
    /// Dense mode rows.
    pub(crate) modes: TinyMap<ModeIndex, ModeImage>,
    /// Dense execution scopes.
    pub(crate) scopes: TinyMap<ScopeIndex, ScopeImage>,
    /// Flattened trigger entries.
    pub(crate) reaction_triggers: Box<[LevelReactionImage]>,
    /// Flattened reaction use ports.
    pub(crate) reaction_use_ports: Box<[PortIndex]>,
    /// Flattened reaction effect ports.
    pub(crate) reaction_effect_ports: Box<[PortIndex]>,
    /// Flattened reaction actions.
    pub(crate) reaction_actions: Box<[ActionIndex]>,
    /// Flattened reaction modes.
    pub(crate) reaction_modes: Box<[ModeIndex]>,
    /// Flattened scope descendants.
    pub(crate) scope_descendants: Box<[ScopeIndex]>,
    /// Flattened scope logical actions.
    pub(crate) scope_logical_actions: Box<[ActionIndex]>,
    /// Flattened scope timer startups.
    pub(crate) scope_timer_startups: Box<[TimerStartupImage]>,
    /// Flattened scope reset reactions.
    pub(crate) scope_reset_reactions: Box<[LevelReactionImage]>,
    /// Flattened scope startup reactions.
    pub(crate) scope_startup_reactions: Box<[LifecycleReactionImage]>,
    /// Flattened scope shutdown reactions.
    pub(crate) scope_shutdown_reactions: Box<[LifecycleReactionImage]>,
    /// Global startup actions.
    pub(crate) startup_actions: Box<[TimerStartupImage]>,
    /// Global timer startup actions.
    pub(crate) timer_startup_actions: Box<[TimerStartupImage]>,
    /// Global shutdown reactions.
    pub(crate) shutdown_reactions: Box<[LifecycleReactionImage]>,
    /// Actions populated before shutdown.
    pub(crate) shutdown_actions: Box<[ActionIndex]>,
    /// Scheduler-boundary routes.
    pub(crate) routes: TinyMap<RouteIndex, OwnedRouteImage>,
    /// Dense runtime binding rows.
    pub(crate) binding_images: TinyMap<BindingSlotIndex, OwnedBindingImage>,
    /// Stable required binding descriptions.
    pub(crate) required_bindings: RequiredBindings,
    /// Mutable storage and workspace bounds.
    pub(crate) storage_bounds: StorageBounds,
}

/// Host-owned scheduler route retaining its typed stable boundary identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OwnedRouteImage {
    /// Stable scheduler-boundary identity.
    pub(crate) boundary: BoundaryId,
    /// Dense Enclave-local port attached to the boundary.
    pub(crate) local_port: PortIndex,
    /// Whether events enter or leave through the local port.
    pub(crate) direction: crate::runtime::image::RouteDirection,
    /// Clock domain used to interpret the route delay.
    pub(crate) timing_domain: crate::runtime::image::TimingDomain,
    /// Route delay in nanoseconds.
    pub(crate) delay_nanos: u64,
}

impl OwnedRouteImage {
    /// Borrows the owned route as a target-facing runtime image row.
    fn image<'a>(&self, boundary: &'a str) -> RouteImage<'a> {
        RouteImage::new(
            runtime_image::BoundaryId::new(boundary),
            self.local_port,
            self.direction,
            self.timing_domain,
            self.delay_nanos,
        )
    }
}

/// Host-owned canonical implementation-binding identity and its required kind.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OwnedBindingImage {
    /// Stable binding identity retained by host-owned storage.
    id: BindingSlotId<OwnedBindingSlotMarker>,
    /// Runtime implementation category required at the binding slot.
    kind: BindingKind,
}

/// Marker separating host-owned binding identities from borrowed runtime identities.
#[derive(Debug)]
struct OwnedBindingSlotMarker;

impl OwnedBindingImage {
    /// Creates a host-owned binding image row from its stable identity and kind.
    pub(crate) fn new(id: impl AsRef<str>, kind: BindingKind) -> Self {
        Self {
            id: BindingSlotId::new(id)
                .expect("binding image identities are built from canonical stable paths"),
            kind,
        }
    }

    /// Borrows the owned binding as a target-facing runtime image row.
    fn image<'a>(&self, id: &'a str) -> RequiredBindingImage<'a> {
        RequiredBindingImage::new(runtime_image::BindingSlotId::new(id), self.kind)
    }
}

impl OwnedEnclaveImage {
    /// Returns the stable Enclave identity.
    pub fn id(&self) -> &StableEnclaveId {
        &self.id
    }

    /// Returns the required payload bindings.
    pub fn required_bindings(&self) -> &RequiredBindings {
        &self.required_bindings
    }

    fn route_identity_text(&self) -> Vec<String> {
        self.routes
            .values()
            .map(|route| route.boundary.to_canonical_string())
            .collect()
    }

    fn route_images<'a>(&self, identities: &'a [String]) -> TinyMap<RouteIndex, RouteImage<'a>> {
        self.routes
            .values()
            .zip(identities)
            .map(|(route, boundary)| route.image(boundary))
            .collect()
    }

    fn binding_identity_text(&self) -> Vec<String> {
        self.binding_images
            .values()
            .map(|binding| binding.id.path().to_string())
            .collect()
    }

    fn binding_rows<'a>(
        &self,
        identities: &'a [String],
    ) -> TinyMap<BindingSlotIndex, RequiredBindingImage<'a>> {
        self.binding_images
            .values()
            .zip(identities)
            .map(|(binding, id)| binding.image(id))
            .collect()
    }

    fn image_with_rows<'a>(
        &'a self,
        enclave_id: &'a str,
        routes: &'a TinyMap<RouteIndex, RouteImage<'a>>,
        bindings: &'a TinyMap<BindingSlotIndex, RequiredBindingImage<'a>>,
    ) -> EnclaveImage<'a> {
        EnclaveImage {
            enclave_id: runtime_image::EnclaveId::new(enclave_id),
            reactors: self.reactors.as_view(),
            actions: self.actions.as_view(),
            ports: self.ports.as_view(),
            reactions: self.reactions.as_view(),
            modes: self.modes.as_view(),
            scopes: self.scopes.as_view(),
            reaction_triggers: &self.reaction_triggers,
            reaction_use_ports: &self.reaction_use_ports,
            reaction_effect_ports: &self.reaction_effect_ports,
            reaction_actions: &self.reaction_actions,
            reaction_modes: &self.reaction_modes,
            scope_descendants: &self.scope_descendants,
            scope_logical_actions: &self.scope_logical_actions,
            scope_timer_startups: &self.scope_timer_startups,
            scope_reset_reactions: &self.scope_reset_reactions,
            scope_startup_reactions: &self.scope_startup_reactions,
            scope_shutdown_reactions: &self.scope_shutdown_reactions,
            startup_actions: &self.startup_actions,
            timer_startup_actions: &self.timer_startup_actions,
            shutdown_reactions: &self.shutdown_reactions,
            shutdown_actions: &self.shutdown_actions,
            routes: routes.as_view(),
            required_bindings: bindings.as_view(),
            storage_bounds: self.storage_bounds,
        }
    }

    /// Materializes the target-facing borrowed image during `f`.
    pub fn with_image<T>(&self, f: impl FnOnce(EnclaveImage<'_>) -> T) -> T {
        let enclave_id = self.id.to_canonical_string();
        let route_identities = self.route_identity_text();
        let routes = self.route_images(&route_identities);
        let binding_identities = self.binding_identity_text();
        let bindings = self.binding_rows(&binding_identities);
        f(self.image_with_rows(&enclave_id, &routes, &bindings))
    }

    /// Validates and exposes the borrowed scheduler image view during `f`.
    pub fn with_view<T>(
        &self,
        f: impl FnOnce(EnclaveImageView<'_>) -> T,
    ) -> Result<T, CompiledDeploymentValidationError> {
        self.with_image(|image| {
            EnclaveImageView::new(&image)
                .map(f)
                .map_err(CompiledDeploymentValidationError::from_image)
        })
    }

    /// Returns this Enclave's declared storage bounds without materializing image rows.
    pub const fn storage_bounds(&self) -> StorageBounds {
        self.storage_bounds
    }
}

/// Heap-backed immutable image for one Federate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnedFederateImage {
    /// Stable Federate identity.
    pub(crate) id: FederateId,
    /// Selected compilation target.
    pub(crate) target: TargetTriple,
    /// Selected runtime backend.
    pub(crate) runtime: RuntimeBackendId,
    /// Deployment-wide span of canonically ordered owned Enclaves.
    pub(crate) enclaves: IndexSpan<EnclaveIndex>,
}

impl OwnedFederateImage {
    /// Returns the stable Federate identity.
    pub fn id(&self) -> &FederateId {
        &self.id
    }

    /// Returns the selected compilation target.
    pub fn target(&self) -> &TargetTriple {
        &self.target
    }

    /// Returns the selected runtime backend.
    pub fn runtime(&self) -> &RuntimeBackendId {
        &self.runtime
    }

    /// Returns the deployment-wide span of owned Enclaves.
    pub const fn enclaves(&self) -> IndexSpan<EnclaveIndex> {
        self.enclaves
    }
}

/// Backend-neutral global federation structure in stable identities.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GlobalFederationImage {
    /// Federates in canonical identity order.
    pub(crate) members: Box<[FederateId]>,
    /// Canonical cross-Federate routes retained independently of any backend projection.
    pub(crate) edges: Box<[super::federation::FederationEdge]>,
}

impl GlobalFederationImage {
    /// Returns canonical federation members.
    pub fn members(&self) -> &[FederateId] {
        &self.members
    }

    /// Returns canonical cross-Federate edges, including parallel routes.
    pub fn edges(&self) -> &[super::federation::FederationEdge] {
        &self.edges
    }
}

/// Heap-backed canonical result of deployment lowering.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnedCompiledDeployment {
    /// Backend-neutral global federation structure.
    pub(crate) federation: GlobalFederationImage,
    /// Complete compiled Federate table keyed by deployment-wide dense identity.
    pub(crate) federates: TinyMap<FederateIndex, OwnedFederateImage>,
    /// Complete compiled Enclave table keyed by deployment-wide dense identity.
    pub(crate) enclaves: TinyMap<EnclaveIndex, OwnedEnclaveImage>,
    /// Selected coordination projection.
    pub(crate) coordination: OwnedCoordinationProjection,
}

/// A borrowed host-side projection for one deployment-wide Federate.
#[derive(Debug)]
pub struct FederateSlice<'a> {
    /// Deployment-wide dense identity of the selected Federate.
    federate: FederateIndex,
    /// Host-owned metadata for the selected Federate.
    image: &'a OwnedFederateImage,
    /// Borrowed deployment table segment owned by the selected Federate.
    enclaves: &'a [OwnedEnclaveImage],
}

/// A failure while projecting one Federate from an owned compiled deployment.
#[derive(Debug, Eq, PartialEq, thiserror::Error)]
pub enum FederateSliceError {
    /// The requested dense Federate key is outside the complete deployment image.
    #[error("compiled Federate {federate:?} does not exist")]
    FederateNotFound {
        /// Requested deployment-wide Federate identity.
        federate: FederateIndex,
    },
    /// Root-format slice metadata exceeds the runtime image's representable domain.
    #[error(transparent)]
    ImageValidation(
        /// Runtime-image validation failure.
        #[from]
        ImageValidationError<'static>,
    ),
}

impl FederateSlice<'_> {
    /// Returns the selected deployment-wide Federate index.
    #[must_use]
    pub const fn federate(&self) -> FederateIndex {
        self.federate
    }

    /// Returns the selected Federate's stable identity.
    #[must_use]
    pub fn id(&self) -> &FederateId {
        &self.image.id
    }

    /// Returns the selected Federate's target triple.
    #[must_use]
    pub fn target(&self) -> &TargetTriple {
        &self.image.target
    }

    /// Returns the selected Federate's runtime backend identity.
    #[must_use]
    pub fn runtime(&self) -> &RuntimeBackendId {
        &self.image.runtime
    }

    /// Returns the selected Federate's deployment-global Enclave range.
    #[must_use]
    pub const fn enclave_range(&self) -> IndexSpan<crate::runtime::image::EnclaveIndex> {
        self.image.enclaves
    }

    /// Returns selected Enclave images in canonical identity order.
    #[must_use]
    pub fn enclaves(&self) -> &[OwnedEnclaveImage] {
        self.enclaves
    }
}

/// An owned validation failure for a host-backed compiled deployment.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("invalid compiled deployment image: {message}")]
pub struct CompiledDeploymentValidationError {
    /// Runtime image validation detail copied out of temporary aggregate storage.
    message: Box<str>,
}

impl CompiledDeploymentValidationError {
    fn from_image(error: ImageValidationError<'_>) -> Self {
        Self {
            message: error.to_string().into_boxed_str(),
        }
    }
}

impl OwnedCompiledDeployment {
    /// Returns the backend-neutral federation structure.
    pub fn federation(&self) -> &GlobalFederationImage {
        &self.federation
    }

    /// Returns the complete Federate table in canonical dense-key order.
    pub fn federates(&self) -> &TinyMap<FederateIndex, OwnedFederateImage> {
        &self.federates
    }

    /// Returns the complete Enclave table in canonical dense-key order.
    pub fn enclaves(&self) -> &TinyMap<EnclaveIndex, OwnedEnclaveImage> {
        &self.enclaves
    }

    /// Returns the selected coordination projection.
    pub fn with_coordination<T>(
        &self,
        f: impl FnOnce(crate::runtime::image::CoordinationProjection<'_>) -> T,
    ) -> T {
        self.coordination.with_image(f)
    }

    /// Borrows one Federate and its Enclaves as an immutable deployment-slice projection.
    ///
    /// The selected [`FederateImage`] retains its complete-root Enclave range so generated
    /// launchers preserve deployment-wide [`crate::runtime::image::EnclaveIndex`] values.
    pub fn federate_slice(
        &self,
        federate: FederateIndex,
    ) -> Result<FederateSlice<'_>, FederateSliceError> {
        let candidate = self
            .federates
            .get(federate)
            .ok_or(FederateSliceError::FederateNotFound { federate })?;

        let enclaves = self.enclaves.get_span(candidate.enclaves).ok_or(
            ImageValidationError::OwnershipMismatch {
                table: "federates",
                index: federate.as_u32(),
                field: "enclaves",
            },
        )?;
        Ok(FederateSlice {
            federate,
            image: candidate,
            enclaves,
        })
    }

    /// Materializes the complete target-facing deployment image during `f`.
    ///
    /// The callback keeps temporary borrowed identity and image rows alive without
    /// requiring a self-referential owned cache.
    pub fn with_image<T>(
        &self,
        f: impl FnOnce(crate::runtime::image::CompiledDeploymentImage<'_>) -> T,
    ) -> T {
        let owned_enclaves = self.enclaves.values().collect::<Vec<_>>();
        let enclave_ids = owned_enclaves
            .iter()
            .map(|enclave| enclave.id.to_canonical_string())
            .collect::<Vec<_>>();
        let route_identities = owned_enclaves
            .iter()
            .map(|enclave| enclave.route_identity_text())
            .collect::<Vec<_>>();
        let route_rows = owned_enclaves
            .iter()
            .zip(&route_identities)
            .map(|(enclave, identities)| enclave.route_images(identities))
            .collect::<Vec<_>>();
        let binding_identities = owned_enclaves
            .iter()
            .map(|enclave| enclave.binding_identity_text())
            .collect::<Vec<_>>();
        let binding_rows = owned_enclaves
            .iter()
            .zip(&binding_identities)
            .map(|(enclave, identities)| enclave.binding_rows(identities))
            .collect::<Vec<_>>();
        let enclaves = owned_enclaves
            .iter()
            .zip(&route_rows)
            .zip(&binding_rows)
            .zip(&enclave_ids)
            .map(|(((enclave, routes), bindings), id)| {
                enclave.image_with_rows(id, routes, bindings)
            })
            .collect::<Vec<_>>();
        let federates = self
            .federates
            .values()
            .map(|federate| {
                FederateImage::new(
                    runtime_image::FederateId::new(federate.id.as_str()),
                    runtime_image::TargetId::new(federate.target.as_str()),
                    runtime_image::RuntimeBackendId::new(federate.runtime.as_str()),
                    federate.enclaves,
                )
            })
            .collect::<Vec<_>>();
        let invalid_federate = FederateIndex::new(u32::MAX);
        let federate_index = |id: &FederateId| {
            self.federates
                .iter()
                .find_map(|(index, federate)| (federate.id == *id).then_some(index))
                .unwrap_or(invalid_federate)
        };
        let members = self
            .federation
            .members
            .iter()
            .map(federate_index)
            .collect::<Vec<_>>();
        let edge_ids = self
            .federation
            .edges
            .iter()
            .map(|edge| edge.id().to_canonical_string())
            .collect::<Vec<_>>();
        let edges = self
            .federation
            .edges
            .iter()
            .zip(&edge_ids)
            .map(|(edge, boundary)| {
                crate::runtime::image::FederationEdgeImage::new(
                    runtime_image::BoundaryId::new(boundary),
                    federate_index(edge.source()),
                    federate_index(edge.target()),
                    edge.delay().as_nanos(),
                )
            })
            .collect::<Vec<_>>();

        self.coordination.with_image(|coordination| {
            f(crate::runtime::image::CompiledDeploymentImage {
                federation: runtime_image::GlobalFederationImage::new(&members, &edges),
                federates: TinyMapView::new(&federates),
                enclaves: TinyMapView::new(&enclaves),
                coordination,
            })
        })
    }

    /// Validates the complete target-facing deployment hierarchy.
    pub fn validate(&self) -> Result<(), CompiledDeploymentValidationError> {
        self.with_image(|image| {
            crate::runtime::image::CompiledDeploymentView::new(&image)
                .map(|_| ())
                .map_err(CompiledDeploymentValidationError::from_image)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::{ComponentInstanceId, ImplementationId, StablePath};
    use crate::descriptor::{ReactionSlotId, ReactorSlotId};
    use crate::runtime::image::{IndexSpan, SliceRange, StateSlotIndex};

    #[test]
    fn direct_binding_symbols_reversibly_escape_descriptor_slots() {
        let reactor = ReactorSlotId::new("Match/loop").unwrap();
        let reaction = ReactionSlotId::new("Match/move").unwrap();
        let generated = ReactionSlotId::new("Match/#g1").unwrap();
        let keyword = ReactionSlotId::new("Match/type").unwrap();
        let raw_identifier = ReactionSlotId::from_path(
            StablePath::from_name("Match")
                .unwrap()
                .append_name("r#type")
                .unwrap(),
        );
        let unicode = ReactionSlotId::new("Mätsch/ø").unwrap();
        let separator = ReactorSlotId::new("Match/left/right").unwrap();
        let escaped_separator = ReactorSlotId::new("Match/left_2fright").unwrap();
        let port = RequiredBinding::Port {
            component: ComponentInstanceId::new("root").unwrap(),
            implementation: ImplementationId::new("root-host").unwrap(),
            port: PortSlotId::new("root/input").unwrap(),
        };
        let action = RequiredBinding::Action {
            component: ComponentInstanceId::new("root").unwrap(),
            implementation: ImplementationId::new("root-host").unwrap(),
            action: ActionSlotId::new("root/tick").unwrap(),
        };

        assert_eq!(
            direct_binding_symbol(BindingKind::StateInitializer, reactor.path()),
            "state_Match_2floop"
        );
        assert_eq!(
            direct_binding_symbol(BindingKind::Reaction, reaction.path()),
            "reaction_Match_2fmove"
        );
        assert_eq!(
            direct_binding_symbol(BindingKind::Reaction, generated.path()),
            "reaction_Match_2f_23g1"
        );
        assert_eq!(
            direct_binding_symbol(BindingKind::Reaction, keyword.path()),
            "reaction_Match_2ftype"
        );
        assert_eq!(
            direct_binding_symbol(BindingKind::Reaction, raw_identifier.path()),
            "reaction_Match_2fr_2523type"
        );
        assert_eq!(
            direct_binding_symbol(BindingKind::Reaction, unicode.path()),
            "reaction_M_c3_a4tsch_2f_c3_b8"
        );
        assert_eq!(
            direct_binding_symbol(BindingKind::StateInitializer, separator.path()),
            "state_Match_2fleft_2fright"
        );
        assert_eq!(
            direct_binding_symbol(BindingKind::StateInitializer, escaped_separator.path()),
            "state_Match_2fleft_5f2fright"
        );
        assert_ne!(
            direct_binding_symbol(BindingKind::StateInitializer, reactor.path()),
            direct_binding_symbol(BindingKind::Reaction, reactor.path())
        );
        assert_eq!(port.kind(), BindingKind::Port);
        assert_eq!(port.symbol(), "port_root_2finput");
        assert_eq!(action.kind(), BindingKind::Action);
        assert_eq!(action.symbol(), "action_root_2ftick");
    }

    #[test]
    fn reused_implementation_slots_keep_logical_instances_distinct() {
        let reactor = ReactorSlotId::new("Match").unwrap();
        let left = RequiredBinding::State {
            component: ComponentInstanceId::new("fleet/left").unwrap(),
            implementation: ImplementationId::new("shared-match").unwrap(),
            reactor,
        };
        let right = RequiredBinding::State {
            component: ComponentInstanceId::new("fleet/right").unwrap(),
            implementation: ImplementationId::new("shared-match").unwrap(),
            reactor: ReactorSlotId::new("Match").unwrap(),
        };

        assert_ne!(left, right);
        assert_eq!(left.symbol(), "state_Match");
        assert_eq!(left.symbol(), right.symbol());
    }

    fn empty_enclave() -> OwnedEnclaveImage {
        OwnedEnclaveImage {
            id: StableEnclaveId::new("vehicle/main").unwrap(),
            reactors: [ReactorImage::new(
                BindingSlotIndex::new(0),
                StateSlotIndex::new(0),
                ScopeIndex::new(0),
                IndexSpan::new(0, 0),
                None,
                None,
            )]
            .into_iter()
            .collect(),
            actions: TinyMap::default(),
            ports: TinyMap::default(),
            reactions: TinyMap::default(),
            modes: TinyMap::default(),
            scopes: [ScopeImage::new(
                None,
                ReactorIndex::new(0),
                None,
                SliceRange::new(0, 1),
                SliceRange::new(0, 0),
                SliceRange::new(0, 0),
                SliceRange::new(0, 0),
                SliceRange::new(0, 0),
                SliceRange::new(0, 0),
            )]
            .into_iter()
            .collect(),
            reaction_triggers: Box::default(),
            reaction_use_ports: Box::default(),
            reaction_effect_ports: Box::default(),
            reaction_actions: Box::default(),
            reaction_modes: Box::default(),
            scope_descendants: Box::new([ScopeIndex::new(0)]),
            scope_logical_actions: Box::default(),
            scope_timer_startups: Box::default(),
            scope_reset_reactions: Box::default(),
            scope_startup_reactions: Box::default(),
            scope_shutdown_reactions: Box::default(),
            startup_actions: Box::default(),
            timer_startup_actions: Box::default(),
            shutdown_reactions: Box::default(),
            shutdown_actions: Box::default(),
            routes: TinyMap::default(),
            binding_images: [OwnedBindingImage::new(
                "state/vehicle/main",
                BindingKind::StateInitializer,
            )]
            .into_iter()
            .collect(),
            required_bindings: RequiredBindings {
                entries: Box::new([RequiredBinding::State {
                    component: ComponentInstanceId::new("vehicle/main").unwrap(),
                    implementation: ImplementationId::new("main-host").unwrap(),
                    reactor: ReactorSlotId::new("Main").unwrap(),
                }]),
            },
            storage_bounds: StorageBounds::new(1, 0, 0, 0, 0, 0),
        }
    }

    #[test]
    fn owned_binding_images_retain_typed_stable_paths() {
        let enclave = empty_enclave();
        let binding = &enclave.binding_images[BindingSlotIndex::new(0)];

        assert_eq!(binding.id.path().to_string(), "state/vehicle/main");
    }

    #[test]
    fn owned_deployment_validates_the_complete_borrowed_hierarchy() {
        let deployment = OwnedCompiledDeployment {
            federation: GlobalFederationImage {
                members: vec![FederateId::new("host").unwrap()].into_boxed_slice(),
                edges: Box::default(),
            },
            federates: vec![OwnedFederateImage {
                id: FederateId::new("host").unwrap(),
                target: TargetTriple::new("x86_64-unknown-linux-gnu").unwrap(),
                runtime: RuntimeBackendId::new("native").unwrap(),
                enclaves: IndexSpan::new(0, 1),
            }]
            .into_iter()
            .collect(),
            enclaves: vec![empty_enclave()].into_iter().collect(),
            coordination: OwnedCoordinationProjection::Local,
        };

        deployment.with_image(|image| {
            assert_eq!(image.federates.len(), 1);
            assert_eq!(image.enclaves.len(), 1);
            assert_eq!(image.federation.members, &[FederateIndex::new(0)]);
            assert_eq!(image.federates[FederateIndex::new(0)].id().as_str(), "host");
        });
        deployment.validate().unwrap();
        let invalid = OwnedCompiledDeployment {
            federation: GlobalFederationImage {
                members: Box::default(),
                edges: Box::default(),
            },
            ..deployment.clone()
        };
        assert!(invalid.validate().is_err());
        let federates: &TinyMap<FederateIndex, OwnedFederateImage> = deployment.federates();
        assert_eq!(
            federates.keys().collect::<Vec<_>>(),
            vec![FederateIndex::new(0)]
        );
        let enclave = &deployment.enclaves()[EnclaveIndex::new(0)];
        let reactor = enclave.reactors.get(ReactorIndex::new(0)).unwrap();
        assert_eq!(reactor.state_binding(), BindingSlotIndex::new(0));
        enclave
            .with_view(|view| {
                assert_eq!(view.enclave_id().as_str(), "vehicle/main");
                assert_eq!(
                    view.reactors()[ReactorIndex::new(0)].state_binding(),
                    BindingSlotIndex::new(0)
                );
                assert_eq!(
                    view.required_binding_id(BindingSlotIndex::new(0)).as_str(),
                    "state/vehicle/main"
                );
            })
            .unwrap();
    }
}
