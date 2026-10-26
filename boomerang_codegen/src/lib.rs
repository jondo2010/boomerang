//! Compile explicit Boomerang deployments into Rust scheduler tables and typed payload bindings.
//!
//! [`Compilation`] uses `boomerang_builder` for semantic validation and lowering.
//! The application supplies topology, deployment configuration, target facts, and implementation selections.
//! The compiler performs no package discovery, environment access, filesystem access, or process execution.
//!
//! # Dependency configuration
//!
//! The default feature set is empty.
//! Enable `cargo-build` on the build dependency to publish generated source through Cargo's `OUT_DIR`.
//! The target application requires `boomerang_runtime`, `boomerang_tinymap` under the alias `tinymap`, and its selected component payloads.
//! Central RTI target packages also require `boomerang_central_rti` and `boomerang_federated`.
//! Keep `boomerang_builder` and `boomerang_codegen` in build dependencies unless application code uses their APIs directly.
//!
//! Use Cargo resolver 2 or newer to isolate build dependencies from normal dependencies.
//! For component dependencies, enable metadata features in build dependencies.
//! Enable payload features in normal dependencies.
//! Set `default-features = false` where default features introduce implementation dependencies into the host build.
//! Cargo aliases determine Rust paths; [`ImplementationId`] determines semantic implementation identity.
//!
//! A single-package application must not depend on itself in build dependencies.
//! Share its component declaration through a declarative macro or source file.
//! Invoke `boomerang::component_metadata!` from the build script.
//! Invoke `boomerang::component_payload!` from the target application.
//! Keep implementation helpers and reaction bodies inside the declaration so metadata emission omits them.
//! Imports, attributes, and dependencies outside the declaration must also respect host/target isolation.
//! The checked-in `boomerang_codegen/tests/fixtures/native` application demonstrates this arrangement.
//! From the repository root, run:
//!
//! ```sh
//! cargo run --manifest-path boomerang_codegen/tests/fixtures/native/Cargo.toml --locked
//! ```
//!
//! # Compilation contract
//!
//! [`DeploymentConfiguration`] supplies the deployment name, placements, federate configurations, coordination selection, and boundary bindings.
//! Register each implementation with [`register_component!`].
//! Use [`Compilation::register`] for explicit descriptors or custom payload exports.
//! Supply its host descriptor and target payload module path independently.
//! Select an implementation for every component instance with [`Compilation::select`].
//! Multiple instances can select the same registration.
//! Duplicate registrations and selections return errors without replacing existing entries.
//! Registration order does not affect output.
//! The builder validates component contracts and versions.
//!
//! [`Compilation::compile`] returns an owned validated deployment image and Rust source for the selected federate.
//! Builder construction validates the complete immutable image once. Backends must not repeat root validation.
//! The selected federate's configured target must equal [`TargetContext::triple`].
//! The compiler never substitutes the host target.
//! For Cargo builds, pass the complete target context from `CargoBuild::target`, including captured `CARGO_CFG_*` facts.
//!
//! [`PayloadExports::new`] accepts a Rust module path without generic arguments, expressions, macros, or trailing source.
//! Invalid paths fail during host generation.
//! One generated assertion per selected implementation compares metadata and target payload descriptor fingerprints.
//! The fingerprint includes the macro ABI version; emitters must retain this compatibility check.
//! Missing target exports and incompatible payloads fail during target compilation.
//! Each reaction binding associates its image-owned `BindingSlotIndex` directly with a payload adapter export.
//! Payload macros own state downcasts, reference extraction, declaration order, and generic parameters.
//! This compiler retains instance indices, routes, and initializer capability mappings.
//! Each compilation owns its implementation catalog.
//! Compilation moves topology, deployment selections, and descriptors through resolution and lowering.
//! Codegen prepares expected descriptor fingerprints before lowering consumes the catalog.
//! Rendering uses these fingerprints and payload paths without descriptor access.
//! Component selections contain catalog identities, and Rust export paths stay in a codegen side table.
//! Lowering resolves initializer actions to descriptor-local payload slots and image-owned action indices.
//! Rendering must consume this plan without descriptor or action-row reconstruction.
//! Reused exports require separate instance storage.
//!
//! ```
//! use boomerang_codegen::{PayloadExports, TargetContext};
//!
//! let _payload = PayloadExports::new("crate::controller::__boomerang::payload")?;
//! assert!(PayloadExports::new("crate::controller::<u32>").is_err());
//! let target = TargetContext::new("aarch64-unknown-linux-gnu")?;
//! assert_eq!(target.triple().as_str(), "aarch64-unknown-linux-gnu");
//! # Ok::<(), boomerang_codegen::CompilationError>(())
//! ```
//!
//! [`DeploymentInputs::lower`] accepts resolved implementation bindings for callers that already perform selection.
//! The [`render`] module uses validated images, expected fingerprints, payload paths, and explicit [`render::BindingConfiguration`].
//!
//! # Build script
//!
//! Enable `cargo-build` for this integration.
//! The following fragment assumes application-defined topology, configuration, component instances, and implementation identifiers:
//!
//! ```rust,ignore
//! use boomerang_codegen::{cargo_build::CargoBuild, Compilation, register_component};
//!
//! let mut cargo = CargoBuild::from_env()?;
//! cargo.track_file("declaration.rs")?;
//! cargo.track_file("topology.rs")?;
//! cargo.track_env("APP_VARIANT")?;
//! let mut compilation = Compilation::new(
//!     topology(), deployment_configuration(cargo.target().triple()),
//!     selected_federate, cargo.target().clone(),
//! );
//! let implementation = register_component!(
//!     compilation, "controller", metadata = controller, payload = crate::controller
//! )?;
//! compilation.select(first_instance, implementation.clone())?;
//! compilation.select(second_instance, implementation)?;
//! cargo.compile(compilation, &mut std::io::stdout())?;
//! ```
//!
//! `CargoBuild::from_env` requires `TARGET` and `OUT_DIR`.
//! It captures all supplied `CARGO_CFG_*` values.
//! `from_values` and `from_context` accept explicit inputs without changes to the process environment.
//! Declare every file or directory read for topology construction with `track_file`.
//! Declare every additional environment input with `track_env`.
//! Pass build-script stdout to `compile` to emit Cargo rerun directives for declared inputs and target facts.
//! Cargo tracks Rust source dependencies through its normal dependency rules.
//! Directive values must not contain newlines or NUL characters.
//! Environment names must be portable identifiers.
//!
//! The adapter owns `OUT_DIR/boomerang.rs` and its temporary publication files.
//! It writes complete source to a temporary file in the same directory before atomic replacement.
//! Identical bytes preserve the final file's modification time.
//! On failure, the adapter removes prior final output.
//! If removal fails, it returns a publication error with the removal cause.
//! It preserves unrelated files and temporary files from interrupted earlier runs.
//! It replaces final symlinks without writing through them.
//! The adapter never invokes Cargo or maintains a compilation cache.
//!
//! # Application execution
//!
//! Include generated source in a module to isolate its imports:
//!
//! ```rust,ignore
//! mod generated { include!(concat!(env!("OUT_DIR"), "/boomerang.rs")); }
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     boomerang_runtime::execute_owned_federate_slice_with_observations(
//!         generated::FEDERATE,
//!         &generated::FEDERATE_VIEW,
//!         generated::generated_bindings(),
//!         boomerang_runtime::Config::default().with_fast_forward(true),
//!         &[],
//!     )?;
//!     Ok(())
//! }
//! ```
//!
//! Generated definitions expose immutable `FEDERATE` and `FEDERATE_VIEW` tables and a callable `generated_bindings` constructor.
//! Inclusion starts no reactions.
//! The application owns `main`, runtime configuration, physical inputs, clocks, transport, and coordinated startup.
//! [`render::BindingConfiguration`] maps component instances to installed physical source keys and records manual-clock installation.
//! Install the matching runtime physical-input configuration and clock before execution.
//! For distributed coordination, `generated_bindings` accepts RTI client bindings and a sink and returns a `Result`.
//! Central RTI compilations also expose immutable coordination tables, identity, codec, and wire admission helpers.
//! [`DeploymentInputs::compile_coordination`] emits these definitions without local scheduler tables or payload bindings.
//! This target-neutral API uses the same builder validation and lowering as selected-federate compilation.
//! Both compilation paths reject unsupported generated-backend capabilities before source publication.
//! The generated central RTI backend requires `tcp` transport and `serde-json` payload capabilities.
//! It requires reliable ordered framing, canonical bounded codecs, propagated stop, best-effort timing, no security, and fail-stop recovery.
//! Runtime selection and startup validation remain application policy.
//! [`DeploymentInputs::lower`] and identity hashing remain independent of generated-backend capability restrictions.
//! Each RTI package owns its Cargo target and toolchain independently from the member targets.
//! [`coordination::coordination_fingerprint`] identifies shared semantics; local image and artifact fingerprints remain distinct.
//! The application owns RTI process startup and runtime configuration.
//! With `cargo-build`, `cargo_build::CargoBuild::compile_coordination` publishes the same fixed `OUT_DIR/boomerang.rs` file.
//!
//! # Diagnostics
//!
//! [`CompilationError`] exposes a [`Stage`], [`DiagnosticCode`], and [`Diagnostic`].
//! Context identifies the deployment and relevant component, implementation, or boundary when available.
//! [`std::error::Error::source`] retains original builder and I/O errors when available.
#[cfg(feature = "cargo-build")]
pub mod cargo_build;
mod compilation;
pub mod coordination;
mod diagnostic;
pub mod render;
pub use compilation::*;
pub use diagnostic::*;

/// A parsed Rust module path to payload exports. This is not a semantic identity.
#[derive(Clone)]
pub struct PayloadExports {
    /// Parsed target-side Rust namespace used to reference concrete payload exports.
    pub(crate) path: syn::Path,
}
impl PayloadExports {
    /// Parse a Cargo-native path such as `crate::controller::__boomerang::payload`.
    /// Generic arguments, expressions, macros, and trailing source are rejected.
    /// The namespace must export a `Payload` marker that implements
    /// [`ComponentPayload`](boomerang_runtime::binding::ComponentPayload).
    pub fn new(path: impl AsRef<str>) -> Result<Self, CompilationError> {
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
        Ok(Self { path })
    }
}

impl std::fmt::Debug for PayloadExports {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use quote::ToTokens;
        f.debug_struct("PayloadExports")
            .field("path", &self.path.to_token_stream().to_string())
            .finish()
    }
}

/// Semantic implementation identity returned by [`register_component!`].
pub use boomerang_builder::compiler::ImplementationId;

/// Register host metadata with an explicit target component module path.
///
/// Returns a registration result containing the implementation identity for [`Compilation::select`].
/// The metadata path resolves in the host build. The payload path remains target-side tokens.
/// Both paths identify public component modules, without generated namespace suffixes.
/// The macro evaluates the compilation and identity expressions once.
/// Validation returns [`CompilationError`] without returning from the enclosing function.
/// Identity errors retain their original cause through [`std::error::Error::source`].
/// Duplicate registrations retain the original registration and its diagnostic context.
///
/// ```rust,ignore
/// let implementation = boomerang_codegen::register_component!(
///     compilation, "sensor", metadata = sensor, payload = physical_input::sensor
/// )?;
/// compilation.select(sensor_instance, implementation)?;
/// ```
#[macro_export]
macro_rules! register_component {
    ($compilation:expr, $implementation:expr, metadata = $metadata:path, payload = $payload:path $(,)?) => {{
        (|| -> ::core::result::Result<$crate::ImplementationId, $crate::CompilationError> {
            let compilation = &mut $compilation;
            let implementation = $crate::ImplementationId::new($implementation)?;
            use $metadata as component_metadata;
            let payload = $crate::PayloadExports::new(::core::concat!(
                ::core::stringify!($payload),
                "::__boomerang::payload"
            ))?;
            compilation
                .register(
                    implementation.clone(),
                    component_metadata::__boomerang::metadata::descriptor(),
                    payload,
                )
                .map(|()| implementation)
        })()
    }};
}
