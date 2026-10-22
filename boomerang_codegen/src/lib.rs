//! Compile explicit Boomerang deployments without Cargo discovery or process execution.
//!
//! [`Compilation`] combines a topology, deployment configuration, target context, and
//! explicit component selections. It returns a validated owned image and Rust definitions.
//! Generated `FEDERATE`, `FEDERATE_VIEW`, and `generated_bindings()` are inert until the
//! application passes them to the runtime. Put the generated file in a module to isolate
//! its imports. Applications own `main`, runtime configuration, physical input installation,
//! and coordinated transport startup.
//!
//! Enable `cargo-build` for the `cargo_build` module, a build-script adapter that tracks declared
//! inputs and publishes `boomerang.rs` in Cargo's `OUT_DIR`. The default feature set is empty.
//! See the crate README and the checked-in `tests/fixtures/native` application for complete
//! metadata/payload and build-script examples.
#[cfg(feature = "cargo-build")]
pub mod cargo_build;
mod compilation;
mod diagnostic;
pub mod render;
pub use compilation::*;
pub use diagnostic::*;

use boomerang_builder::ComponentDescriptor;

/// How a payload namespace exports its runtime compatibility manifest.
#[derive(Debug, Clone, Copy)]
pub enum ManifestStyle {
    /// Cargo-native and module legacy exports: `binding_manifest()`.
    Function,
    /// Legacy crate exports: `BINDING_MANIFEST`.
    Constant,
}

/// A parsed Rust module path to payload exports. This is not a semantic identity.
#[derive(Clone)]
pub struct PayloadExports {
    pub(crate) path: syn::Path,
    pub(crate) manifest: ManifestStyle,
}
impl PayloadExports {
    /// Parse a Cargo-native path such as `crate::controller::__boomerang::payload`.
    /// Generic arguments, expressions, macros, and trailing source are rejected.
    pub fn new(path: impl AsRef<str>) -> Result<Self, CompilationError> {
        Self::with_manifest(path, ManifestStyle::Function)
    }
    /// Adapt an explicitly selected export convention, including legacy CLI payloads.
    pub fn with_manifest(
        path: impl AsRef<str>,
        manifest: ManifestStyle,
    ) -> Result<Self, CompilationError> {
        let path = path.as_ref();
        let parsed = syn::parse_str::<syn::Path>(path).ok().filter(|path| {
            !path.segments.is_empty()
                && path
                    .segments
                    .iter()
                    .all(|s| matches!(s.arguments, syn::PathArguments::None))
        });
        let path = parsed.ok_or_else(|| {
            CompilationError::new(
                Stage::Registration,
                DiagnosticCode::InvalidPayloadPath,
                format!("invalid payload export path '{path}'"),
            )
        })?;
        Ok(Self { path, manifest })
    }
}

/// One registered implementation's independently obtained descriptor and target exports.
#[derive(Debug, Clone)]
pub struct ComponentRegistration {
    pub descriptor: ComponentDescriptor,
    pub payload: PayloadExports,
}

impl std::fmt::Debug for PayloadExports {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use quote::ToTokens;
        f.debug_struct("PayloadExports")
            .field("path", &self.path.to_token_stream().to_string())
            .field("manifest", &self.manifest)
            .finish()
    }
}
