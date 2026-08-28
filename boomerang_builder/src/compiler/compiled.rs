use super::{FederateId, ReactionId, ReactorId, RuntimeBackendId, StableEnclaveId, TargetTriple};
use crate::runtime::image::{
    ActionImage, ActionIndex, BindingKind, CoordinationProjection, EnclaveImage, EnclaveImageView,
    ImageValidationError, LevelReactionImage, LifecycleReactionImage, ModeImage, ModeIndex,
    PortImage, PortIndex, ReactionImage, ReactorImage, RequiredBindingImage, RouteImage,
    ScopeImage, ScopeIndex, StorageBounds, TimerStartupImage,
};

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
        /// Stable reactor identity.
        reactor: ReactorId,
    },
    /// Invokes one user reaction implementation.
    Reaction {
        /// Stable reaction identity.
        reaction: ReactionId,
    },
}

impl RequiredBinding {
    /// Returns the runtime binding category.
    pub const fn kind(&self) -> BindingKind {
        match self {
            Self::State { .. } => BindingKind::StateInitializer,
            Self::Reaction { .. } => BindingKind::Reaction,
        }
    }
}

/// Heap-backed immutable scheduler image for one Enclave.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnedEnclaveImage {
    /// Stable Enclave identity.
    pub(crate) id: StableEnclaveId,
    /// UTF-8 storage for image-local stable identities.
    pub(crate) identity_data: Box<str>,
    /// Enclave identity range in `identity_data`.
    pub(crate) enclave_id: crate::runtime::image::IdentityRange,
    /// Dense reactor rows.
    pub(crate) reactors: Box<[ReactorImage]>,
    /// Dense action rows.
    pub(crate) actions: Box<[ActionImage]>,
    /// Dense port rows.
    pub(crate) ports: Box<[PortImage]>,
    /// Dense reaction rows.
    pub(crate) reactions: Box<[ReactionImage]>,
    /// Dense mode rows.
    pub(crate) modes: Box<[ModeImage]>,
    /// Dense execution scopes.
    pub(crate) scopes: Box<[ScopeImage]>,
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
    pub(crate) routes: Box<[RouteImage]>,
    /// Dense runtime binding rows.
    pub(crate) binding_images: Box<[RequiredBindingImage]>,
    /// Stable required binding descriptions.
    pub(crate) required_bindings: RequiredBindings,
    /// Mutable storage and workspace bounds.
    pub(crate) storage_bounds: StorageBounds,
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

    /// Constructs the borrowed target-facing image aggregate.
    pub fn image(&self) -> EnclaveImage<'_> {
        EnclaveImage {
            identity_data: &self.identity_data,
            enclave_id: self.enclave_id,
            reactors: &self.reactors,
            actions: &self.actions,
            ports: &self.ports,
            reactions: &self.reactions,
            modes: &self.modes,
            scopes: &self.scopes,
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
            routes: &self.routes,
            required_bindings: &self.binding_images,
            storage_bounds: self.storage_bounds,
        }
    }

    /// Validates and returns a borrowed scheduler image view.
    pub fn view(&self) -> Result<EnclaveImageView<'_>, ImageValidationError<'_>> {
        EnclaveImageView::new(&self.image())
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
    /// Canonically ordered owned Enclaves.
    pub(crate) enclaves: Box<[OwnedEnclaveImage]>,
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

    /// Returns owned Enclaves in canonical identity order.
    pub fn enclaves(&self) -> &[OwnedEnclaveImage] {
        &self.enclaves
    }
}

/// Backend-neutral global federation structure in stable identities.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GlobalFederationImage {
    /// Federates in canonical identity order.
    pub(crate) members: Box<[FederateId]>,
}

impl GlobalFederationImage {
    /// Returns canonical federation members.
    pub fn members(&self) -> &[FederateId] {
        &self.members
    }
}

/// Heap-backed canonical result of deployment lowering.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnedCompiledDeployment {
    /// Backend-neutral global federation structure.
    pub(crate) federation: GlobalFederationImage,
    /// Federate-owned compiled image slices.
    pub(crate) federates: Box<[OwnedFederateImage]>,
    /// Selected coordination projection.
    pub(crate) coordination: CoordinationProjection,
}

impl OwnedCompiledDeployment {
    /// Returns the backend-neutral federation structure.
    pub fn federation(&self) -> &GlobalFederationImage {
        &self.federation
    }

    /// Returns Federate images in canonical identity order.
    pub fn federates(&self) -> &[OwnedFederateImage] {
        &self.federates
    }

    /// Returns the selected coordination projection.
    pub const fn coordination(&self) -> CoordinationProjection {
        self.coordination
    }

    /// Validates every target-facing Enclave image.
    pub fn validate(&self) -> Result<(), ImageValidationError<'_>> {
        for federate in &self.federates {
            for enclave in &federate.enclaves {
                enclave.view()?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::image::IdentityRange;

    fn empty_enclave() -> OwnedEnclaveImage {
        OwnedEnclaveImage {
            id: StableEnclaveId::new("vehicle/main").unwrap(),
            identity_data: "vehicle/main".into(),
            enclave_id: IdentityRange::new(0, 12),
            reactors: Box::default(),
            actions: Box::default(),
            ports: Box::default(),
            reactions: Box::default(),
            modes: Box::default(),
            scopes: Box::default(),
            reaction_triggers: Box::default(),
            reaction_use_ports: Box::default(),
            reaction_effect_ports: Box::default(),
            reaction_actions: Box::default(),
            reaction_modes: Box::default(),
            scope_descendants: Box::default(),
            scope_logical_actions: Box::default(),
            scope_timer_startups: Box::default(),
            scope_reset_reactions: Box::default(),
            scope_startup_reactions: Box::default(),
            scope_shutdown_reactions: Box::default(),
            startup_actions: Box::default(),
            timer_startup_actions: Box::default(),
            shutdown_reactions: Box::default(),
            shutdown_actions: Box::default(),
            routes: Box::default(),
            binding_images: Box::default(),
            required_bindings: RequiredBindings {
                entries: Box::default(),
            },
            storage_bounds: StorageBounds::new(0, 0, 0, 0, 0, 0),
        }
    }

    #[test]
    fn owned_deployment_validates_through_the_borrowed_enclave_view() {
        let deployment = OwnedCompiledDeployment {
            federation: GlobalFederationImage {
                members: vec![FederateId::new("host").unwrap()].into_boxed_slice(),
            },
            federates: vec![OwnedFederateImage {
                id: FederateId::new("host").unwrap(),
                target: TargetTriple::new("x86_64-unknown-linux-gnu").unwrap(),
                runtime: RuntimeBackendId::new("native").unwrap(),
                enclaves: vec![empty_enclave()].into_boxed_slice(),
            }]
            .into_boxed_slice(),
            coordination: CoordinationProjection::Local,
        };

        deployment.validate().unwrap();
        let view = deployment.federates()[0].enclaves()[0].view().unwrap();
        assert_eq!(view.enclave_id().as_str(), "vehicle/main");
    }
}
