use crate::{
    render::{self, BindingConfiguration},
    *,
};
use boomerang_builder::{compiler::*, ComponentDescriptor};
use std::{
    collections::{btree_map::Entry, BTreeMap},
    sync::Arc,
};

/// Explicit deployment choices shared by independently compiled application packages.
#[derive(Debug, Clone)]
pub struct DeploymentConfiguration {
    /// Deployment label attached to compilation diagnostics.
    pub name: String,
    /// Assignments from stable placement-group identities to stable federate identities.
    pub placements: Vec<PlacementAssignment>,
    /// Target, runtime backend, and recovery policy for each stable federate identity.
    pub federates: Vec<FederateConfig>,
    /// Local or distributed coordination policy validated by the builder.
    pub coordination: CoordinationSelection,
    /// Bindings from stable logical boundary identities to deployment transport capabilities.
    pub boundaries: Vec<BoundaryBinding>,
}
/// Builder-owned topology and resolved implementation selections. No discovery policy.
pub struct DeploymentInputs {
    /// Target-neutral application structure in stable component and slot identities.
    pub topology: ApplicationTopology,
    /// Explicit deployment policy applied during builder resolution.
    pub configuration: DeploymentConfiguration,
    /// Shared structural descriptors stored once per implementation identity.
    pub catalog: Arc<ImplementationCatalog>,
    /// Component-instance selections containing only catalog identities.
    pub bindings: Vec<ImplementationBinding>,
}
impl DeploymentInputs {
    /// Compile target-neutral central RTI definitions from explicit deployment inputs.
    ///
    /// Builder validation and lowering own deployment semantics.
    /// Boundary hashing uses the original topology without serialization or discovery.
    /// This API selects no member target. The RTI package owns its target and toolchain.
    /// Local coordination returns a render-stage error.
    pub fn compile_coordination(self) -> Result<CoordinationCompilationOutput, CompilationError> {
        let topology = self.topology.clone();
        let name = self.configuration.name.clone();
        let image = self.lower()?;
        let tokens = coordination::render_definitions(&image, &topology)
            .map_err(|e| CompilationError::render(e, &name))?;
        let source = render::format_rust(tokens).map_err(|e| CompilationError::render(e, &name))?;
        Ok(CoordinationCompilationOutput { image, source })
    }
    /// Use the builder's canonical semantic validation, lowering, and image validation.
    pub fn lower(self) -> Result<OwnedCompiledDeployment, CompilationError> {
        let config = self.configuration;
        let resolved = ResolvedDeployment::new(
            self.topology,
            self.catalog,
            self.bindings,
            config.placements,
            config.federates,
            config.coordination,
            config.boundaries,
        )
        .map_err(|e| CompilationError::resolve(e, &config.name))?;
        let image = resolved
            .lower()
            .map_err(|e| CompilationError::lower(e, &config.name))?;
        image.validate().map_err(|e| {
            CompilationError::new(Stage::Validate, DiagnosticCode::Validate, e.to_string())
                .deployment(&config.name)
                .cause(e)
        })?;
        Ok(image)
    }
}
/// Target facts supplied by the caller; never inferred from host `cfg!` values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetContext {
    /// Application target triple checked against the selected federate's configured target.
    triple: TargetTriple,
    /// Captured `CARGO_CFG_*` names and values retained without host-side interpretation.
    cfg: BTreeMap<String, String>,
}
impl TargetContext {
    pub fn new(triple: impl AsRef<str>) -> Result<Self, CompilationError> {
        let triple = TargetTriple::new(triple.as_ref()).map_err(|e| {
            CompilationError::new(Stage::Target, DiagnosticCode::InvalidInput, e.to_string())
                .cause(e)
        })?;
        Ok(Self {
            triple,
            cfg: BTreeMap::new(),
        })
    }
    pub fn triple(&self) -> &TargetTriple {
        &self.triple
    }
    /// Captured `CARGO_CFG_*` facts. Interpretation remains the application's policy.
    pub fn cfg(&self) -> &BTreeMap<String, String> {
        &self.cfg
    }
    #[cfg(feature = "cargo-build")]
    pub(crate) fn with_cfg(mut self, cfg: BTreeMap<String, String>) -> Self {
        self.cfg = cfg;
        self
    }
}
/// Explicit pure compilation request. Registration and selection are separate operations.
#[derive(Debug)]
pub struct Compilation {
    /// Owned target-neutral application structure supplied to builder resolution.
    topology: ApplicationTopology,
    /// Explicit deployment choices shared by all federates in the compiled image.
    configuration: DeploymentConfiguration,
    /// Stable identity of the federate selected for Rust source generation.
    federate: FederateId,
    /// Caller-supplied target facts checked before builder lowering.
    target: TargetContext,
    /// Shared canonical host descriptors keyed by implementation identity.
    catalog: Arc<ImplementationCatalog>,
    /// Rust payload export paths kept outside semantic resolution.
    exports: BTreeMap<ImplementationId, PayloadExports>,
    /// Explicit mapping from stable component-instance identity to registered implementation identity.
    selections: BTreeMap<ComponentInstanceId, ImplementationId>,
    /// Application-installed runtime capabilities used by the generated binding constructor.
    bindings: BindingConfiguration,
}
/// Owned validated image and generated Rust for the selected federate.
#[derive(Debug)]
pub struct CompilationOutput {
    /// Owned, validated deployment image containing all configured federates.
    pub image: OwnedCompiledDeployment,
    /// Formatted Rust tables and binding definitions for the selected federate.
    pub source: String,
}
/// Owned validated deployment image and target-neutral coordination source.
#[derive(Debug)]
pub struct CoordinationCompilationOutput {
    /// Complete validated deployment image used by the mechanical RTI projection.
    pub image: OwnedCompiledDeployment,
    /// Formatted coordination tables and wire definitions without payload bindings or execution.
    pub source: String,
}
impl Compilation {
    pub fn new(
        topology: ApplicationTopology,
        configuration: DeploymentConfiguration,
        federate: FederateId,
        target: TargetContext,
    ) -> Self {
        Self {
            topology,
            configuration,
            federate,
            target,
            catalog: Arc::new(ImplementationCatalog::new()),
            exports: BTreeMap::new(),
            selections: BTreeMap::new(),
            bindings: BindingConfiguration::default(),
        }
    }
    pub fn target(&self) -> &TargetContext {
        &self.target
    }
    /// Configure capability source keys and whether the application installs a manual clock.
    /// Distributed mode is derived from deployment coordination; the application owns startup.
    pub fn binding_configuration(&mut self, configuration: BindingConfiguration) {
        self.bindings = configuration;
    }
    pub fn register(
        &mut self,
        implementation: ImplementationId,
        descriptor: ComponentDescriptor,
        payload: PayloadExports,
    ) -> Result<(), CompilationError> {
        match self.exports.entry(implementation.clone()) {
            Entry::Vacant(entry) => {
                Arc::make_mut(&mut self.catalog).insert(implementation, descriptor);
                entry.insert(payload);
                Ok(())
            }
            Entry::Occupied(_) => Err(CompilationError::new(
                Stage::Registration,
                DiagnosticCode::DuplicateRegistration,
                format!("duplicate implementation '{implementation}'"),
            )
            .deployment(&self.configuration.name)
            .implementation(&implementation)),
        }
    }
    pub fn select(
        &mut self,
        component: ComponentInstanceId,
        implementation: ImplementationId,
    ) -> Result<(), CompilationError> {
        match self.selections.entry(component.clone()) {
            Entry::Vacant(entry) => {
                entry.insert(implementation);
                Ok(())
            }
            Entry::Occupied(_) => Err(CompilationError::new(
                Stage::Registration,
                DiagnosticCode::DuplicateSelection,
                format!("duplicate selection for '{component}'"),
            )
            .deployment(&self.configuration.name)
            .component(&component)
            .implementation(&implementation)),
        }
    }
    /// Compile with no filesystem, environment, process, or publication effects.
    pub fn compile(&self) -> Result<CompilationOutput, CompilationError> {
        let name = &self.configuration.name;
        let federate = self
            .configuration
            .federates
            .iter()
            .find(|f| f.id() == &self.federate)
            .ok_or_else(|| {
                CompilationError::new(
                    Stage::Target,
                    DiagnosticCode::MissingTarget,
                    format!("missing federate '{}'", self.federate),
                )
                .deployment(name)
            })?;
        if federate.target() != self.target.triple() {
            return Err(CompilationError::new(
                Stage::Target,
                DiagnosticCode::TargetMismatch,
                format!(
                    "federate '{}' targets '{}' but target context is '{}'",
                    self.federate,
                    federate.target(),
                    self.target.triple()
                ),
            )
            .deployment(name));
        }
        let bindings = self.selections.iter().map(|(component, implementation)| {
            if !self.catalog.contains_key(implementation) {
                return Err(CompilationError::new(
                    Stage::Registration, DiagnosticCode::MissingRegistration,
                    format!("component '{component}' selects unregistered implementation '{implementation}'"),
                ).deployment(name).component(component).implementation(implementation));
            }
            Ok(ImplementationBinding::new(component.clone(), implementation.clone()))
        }).collect::<Result<Vec<_>, CompilationError>>()?;
        let image = DeploymentInputs {
            topology: self.topology.clone(),
            configuration: self.configuration.clone(),
            catalog: Arc::clone(&self.catalog),
            bindings,
        }
        .lower()?;
        let (index, _) = image
            .federates()
            .iter()
            .find(|(_, f)| f.id() == &self.federate)
            .expect("validated federate selection");
        let slice = image.federate_slice(index).map_err(|e| {
            CompilationError::new(Stage::Render, DiagnosticCode::Render, e.to_string())
                .deployment(name)
                .cause(e)
        })?;
        let mut options = self.bindings.clone();
        options.distributed = matches!(
            self.configuration.coordination,
            CoordinationSelection::Distributed { .. }
        );
        let mut tokens = render::render_definitions(&slice, &self.catalog, &self.exports, &options)
            .map_err(|e| CompilationError::render(e, name))?;
        if image.with_coordination(|projection| {
            matches!(
                projection,
                boomerang_runtime::image::CoordinationProjection::CentralRti(_)
            )
        }) {
            tokens.extend(
                coordination::render_definitions(&image, &self.topology)
                    .map_err(|e| CompilationError::render(e, name))?,
            );
        }
        let source = render::format_rust(tokens).map_err(|e| CompilationError::render(e, name))?;
        Ok(CompilationOutput { image, source })
    }
}
