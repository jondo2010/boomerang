use boomerang_builder::compiler::{
    CompileError, ComponentInstanceId, ImplementationId, RequiredBinding, ResolveError,
};

/// Stage at which compilation or publication failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Registration,
    Target,
    Resolve,
    Lower,
    Render,
    Cargo,
    Publish,
}
/// Machine-readable diagnostic classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticCode {
    InvalidPayloadPath,
    DuplicateRegistration,
    DuplicateSelection,
    MissingRegistration,
    TargetMismatch,
    MissingTarget,
    InvalidInput,
    Resolve,
    Lower,
    Render,
    Io,
}
/// Stable compilation context independent of CLI presentation.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    /// Compiler or publication stage that produced the failure.
    pub stage: Stage,
    /// Machine-readable classification independent of the diagnostic message.
    pub code: DiagnosticCode,
    /// Caller-supplied deployment label, when the failing operation has deployment context.
    pub deployment: Option<String>,
    /// Stable component-instance identity associated with the failure, when available.
    pub component: Option<ComponentInstanceId>,
    /// Semantic implementation identity associated with the failure, when available.
    pub implementation: Option<ImplementationId>,
    /// Failing runtime binding or logical boundary description, when available.
    pub binding: Option<String>,
    /// Human-readable failure detail for display alongside the structured context.
    pub message: String,
}
/// Structured context plus the original builder or I/O error where available.
#[derive(Debug)]
pub struct CompilationError {
    /// Owned structured diagnostic exposed to library callers and presentation adapters.
    pub diagnostic: Box<Diagnostic>,
    /// Original builder, rendering, or I/O error retained for the error source chain.
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
}
impl CompilationError {
    pub(crate) fn new(stage: Stage, code: DiagnosticCode, message: impl Into<String>) -> Self {
        Self {
            diagnostic: Box::new(Diagnostic {
                stage,
                code,
                deployment: None,
                component: None,
                implementation: None,
                binding: None,
                message: message.into(),
            }),
            source: None,
        }
    }
    pub(crate) fn cause(mut self, source: impl std::error::Error + Send + Sync + 'static) -> Self {
        self.source = Some(Box::new(source));
        self
    }
    pub(crate) fn deployment(mut self, name: &str) -> Self {
        self.diagnostic.deployment = Some(name.into());
        self
    }
    pub(crate) fn component(mut self, component: &ComponentInstanceId) -> Self {
        self.diagnostic.component = Some(component.clone());
        self
    }
    pub(crate) fn implementation(mut self, implementation: &ImplementationId) -> Self {
        self.diagnostic.implementation = Some(implementation.clone());
        self
    }
    pub(crate) fn render_binding(error: anyhow::Error, binding: &RequiredBinding) -> Self {
        let mut result = Self::new(Stage::Render, DiagnosticCode::Render, format!("{error:#}"))
            .component(binding.component())
            .implementation(binding.implementation());
        result.diagnostic.binding = Some(binding.symbol());
        result.source = Some(error.into_boxed_dyn_error());
        result
    }
    pub(crate) fn render(error: anyhow::Error, name: &str) -> Self {
        match error.downcast::<Self>() {
            Ok(error) => error.deployment(name),
            Err(error) => {
                let mut result =
                    Self::new(Stage::Render, DiagnosticCode::Render, format!("{error:#}"))
                        .deployment(name);
                result.source = Some(error.into_boxed_dyn_error());
                result
            }
        }
    }
    pub(crate) fn lower(error: CompileError, name: &str) -> Self {
        let mut result =
            Self::new(Stage::Lower, DiagnosticCode::Lower, error.to_string()).deployment(name);
        match &error {
            CompileError::UnboundedResource { component, .. } => {
                result.diagnostic.component = Some(component.clone());
            }
            CompileError::DescriptorRoot {
                component,
                implementation,
                ..
            }
            | CompileError::MissingDescriptorSlot {
                component,
                implementation,
                ..
            }
            | CompileError::MissingInitializerAction {
                component,
                implementation,
                ..
            }
            | CompileError::AmbiguousDescriptorSlot {
                component,
                implementation,
                ..
            } => {
                result.diagnostic.component = Some(component.clone());
                result.diagnostic.implementation = Some(implementation.clone());
                if let CompileError::MissingDescriptorSlot { logical, .. }
                | CompileError::AmbiguousDescriptorSlot { logical, .. } = &error
                {
                    result.diagnostic.binding = Some(logical.clone());
                }
            }
            CompileError::UnsupportedPhysicalFederation { boundary } => {
                result.diagnostic.binding = Some(boundary.to_string());
            }
            CompileError::UnsupportedModeTransition { reaction } => {
                result.diagnostic.binding = Some(reaction.to_string());
            }
            _ => {}
        }
        result.cause(error)
    }
    pub(crate) fn resolve(error: ResolveError, name: &str) -> Self {
        let mut result =
            Self::new(Stage::Resolve, DiagnosticCode::Resolve, error.to_string()).deployment(name);
        match &error {
            ResolveError::DuplicateComponentBinding { component }
            | ResolveError::MissingComponentBinding { component } => {
                result.diagnostic.component = Some(component.clone());
            }
            ResolveError::UnknownComponentBinding {
                component,
                implementation,
            }
            | ResolveError::MissingImplementation {
                component,
                implementation,
            }
            | ResolveError::ContractMismatch {
                component,
                implementation,
                ..
            } => {
                result.diagnostic.component = Some(component.clone());
                result.diagnostic.implementation = Some(implementation.clone());
            }
            ResolveError::DuplicateBoundaryBinding { boundary }
            | ResolveError::MissingBoundaryBinding { boundary }
            | ResolveError::UnexpectedBoundaryBinding { boundary }
            | ResolveError::UnknownBoundaryBinding { boundary } => {
                result.diagnostic.binding = Some(boundary.to_string());
            }
            _ => {}
        }
        result.cause(error)
    }
}
impl std::fmt::Display for CompilationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(deployment) = &self.diagnostic.deployment {
            write!(f, "deployment '{deployment}': ")?;
        }
        write!(
            f,
            "{:?}: {}",
            self.diagnostic.stage, self.diagnostic.message
        )
    }
}
impl std::error::Error for CompilationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source.as_ref().map(|e| &**e as _)
    }
}
