use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use cargo_metadata::{Metadata, MetadataCommand, Package, PackageId};
use thiserror::Error;

use crate::{load_manifest, Federate, ManifestError};

const DESCRIPTOR_FEATURE: &str = "__boomerang_descriptor";
const PAYLOAD_FEATURE: &str = "__boomerang_payload";

/// Reserved Cargo features selecting a package's deployment facets.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FacetFeatures {
    /// Feature selecting host descriptor generation.
    pub descriptor: &'static str,
    /// Feature selecting target payload generation.
    pub payload: &'static str,
}

/// Exact Cargo package containing the application topology entry point.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedTopology {
    /// Opaque package identity reported by Cargo.
    pub id: PackageId,
    /// Cargo package name retained for diagnostics and generated manifests.
    pub name: String,
    /// Absolute path to the selected package manifest.
    pub manifest_path: PathBuf,
    /// Rust path to the topology entry point.
    pub entry: String,
}

/// One exact component implementation package selected for deployment analysis.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedPackage {
    /// Opaque package identity reported by Cargo.
    pub id: PackageId,
    /// Cargo package name retained for diagnostics and generated manifests.
    pub name: String,
    /// Absolute path to the selected package manifest.
    pub manifest_path: PathBuf,
    /// Normal Cargo features selected by `Boomerang.toml`.
    pub features: Vec<String>,
    /// Reserved descriptor and payload features declared by the package.
    pub facets: FacetFeatures,
}

/// Resolved target and runtime configuration for one Federate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedFederate {
    /// Stable placement groups assigned to this Federate.
    pub groups: Vec<String>,
    /// Optional Rust target triple; absence selects the host target.
    pub target: Option<String>,
    /// Optional Rust toolchain selector.
    pub toolchain: Option<String>,
    /// Optional Cargo profile; absence selects Cargo's development profile.
    pub profile: Option<String>,
    /// Runtime backend required by the generated Federate.
    pub runtime: String,
    /// Workspace-relative target JSON resolved to an absolute path.
    pub target_json: Option<PathBuf>,
    /// Workspace-relative Cargo configuration resolved to an absolute path.
    pub cargo_config: Option<PathBuf>,
}

/// Stable identity of the source application's lockfile.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LockfileIdentity {
    /// Absolute path to the source workspace lockfile.
    pub path: PathBuf,
    /// BLAKE3 digest of the exact lockfile bytes.
    pub digest: [u8; 32],
}

/// Cargo-resolved inputs for one named deployment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedWorkspace {
    /// Exact package containing the application topology entry point.
    pub topology: ResolvedTopology,
    /// Component-instance paths mapped to exact implementation packages.
    pub bindings: BTreeMap<String, ResolvedPackage>,
    /// Federate identifiers mapped to resolved build configuration.
    pub federates: BTreeMap<String, ResolvedFederate>,
    /// Identity of the lockfile enforced during Cargo resolution.
    pub lockfile: LockfileIdentity,
}

/// Failure while resolving a deployment against locked Cargo metadata.
#[derive(Debug, Error)]
pub enum WorkspaceError {
    /// The application workspace path could not be canonicalized.
    #[error("failed to resolve application workspace {path}: {source}")]
    WorkspacePath {
        /// Supplied application workspace path.
        path: PathBuf,
        /// Underlying filesystem error.
        #[source]
        source: std::io::Error,
    },
    /// `Boomerang.toml` could not be loaded or validated.
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    /// Locked Cargo metadata could not be obtained.
    #[error("failed to resolve locked Cargo metadata for {manifest}: {source}")]
    Metadata {
        /// Application workspace manifest passed to Cargo.
        manifest: PathBuf,
        /// Cargo invocation or metadata decoding error.
        #[source]
        source: cargo_metadata::Error,
    },
    /// A component implementation selection failed package resolution.
    #[error("deployment '{deployment}' binding '{component}': {source}")]
    Binding {
        /// Named deployment containing the invalid binding.
        deployment: String,
        /// Stable component-instance path selecting the package.
        component: String,
        /// Package or feature resolution failure.
        #[source]
        source: Box<WorkspaceError>,
    },
    /// A selected package is not visible in Cargo metadata.
    #[error("package '{package}' was not found in the application workspace metadata")]
    UnknownPackage {
        /// Package name selected by `Boomerang.toml`.
        package: String,
    },
    /// A visible package is not an application workspace member.
    #[error("package '{package}' must be a member of the application workspace")]
    NonmemberPackage {
        /// Nonmember package selected by `Boomerang.toml`.
        package: String,
    },
    /// A selected package does not declare a required deployment facet.
    #[error("package '{package}' must declare reserved feature '{feature}'")]
    MissingFacet {
        /// Package missing the reserved feature.
        package: String,
        /// Required reserved feature name.
        feature: &'static str,
    },
    /// A binding selects a reserved feature as a normal feature.
    #[error("package '{package}' feature '{feature}' is reserved for cargo-boomerang")]
    ReservedFeature {
        /// Package whose feature selection is invalid.
        package: String,
        /// Reserved feature selected by the manifest.
        feature: String,
    },
    /// A binding selects a feature absent from its package.
    #[error("package '{package}' does not declare selected feature '{feature}'")]
    UnknownFeature {
        /// Package whose feature selection is invalid.
        package: String,
        /// Missing selected feature.
        feature: String,
    },
    /// The source workspace lockfile could not be read.
    #[error("failed to read source workspace lockfile {path}: {source}")]
    Lockfile {
        /// Source workspace lockfile path.
        path: PathBuf,
        /// Underlying filesystem error.
        #[source]
        source: std::io::Error,
    },
}

/// Resolves one deployment through Cargo's locked workspace metadata without compiling packages.
pub fn resolve_workspace(
    workspace: impl AsRef<Path>,
    deployment_name: &str,
) -> Result<ResolvedWorkspace, WorkspaceError> {
    let supplied_workspace = workspace.as_ref();
    let workspace =
        fs::canonicalize(supplied_workspace).map_err(|source| WorkspaceError::WorkspacePath {
            path: supplied_workspace.to_path_buf(),
            source,
        })?;
    let manifest = load_manifest(workspace.join("Boomerang.toml"))?;
    let deployment = manifest.deployment(deployment_name)?;
    let cargo_manifest = workspace.join("Cargo.toml");
    let metadata = locked_metadata(&workspace, &cargo_manifest)?;
    let workspace_root = metadata.workspace_root.clone().into_std_path_buf();

    let topology = resolve_topology(
        &metadata,
        &manifest.topology.package,
        &manifest.topology.entry,
    )?;
    let bindings = deployment
        .bindings
        .iter()
        .map(|(component, binding)| {
            resolve_package(&metadata, &binding.package, &binding.features)
                .map(|package| (component.clone(), package))
                .map_err(|source| WorkspaceError::Binding {
                    deployment: deployment_name.to_owned(),
                    component: component.clone(),
                    source: Box::new(source),
                })
        })
        .collect::<Result<_, _>>()?;
    let federates = deployment
        .federates
        .iter()
        .map(|(name, federate)| (name.clone(), resolve_federate(&workspace_root, federate)))
        .collect();
    let lockfile = lockfile_identity(workspace_root.join("Cargo.lock"))?;

    Ok(ResolvedWorkspace {
        topology,
        bindings,
        federates,
        lockfile,
    })
}

/// Resolves the host-compatible topology package without imposing implementation facets.
fn resolve_topology(
    metadata: &Metadata,
    name: &str,
    entry: &str,
) -> Result<ResolvedTopology, WorkspaceError> {
    let package = workspace_member(metadata, name)?;
    Ok(ResolvedTopology {
        id: package.id.clone(),
        name: package.name.to_string(),
        manifest_path: package.manifest_path.clone().into_std_path_buf(),
        entry: entry.to_owned(),
    })
}

/// Invokes Cargo's metadata command with lockfile updates forbidden.
fn locked_metadata(workspace: &Path, manifest: &Path) -> Result<Metadata, WorkspaceError> {
    let mut command = MetadataCommand::new();
    command
        .current_dir(workspace)
        .manifest_path(manifest)
        .other_options(vec![String::from("--locked")]);
    command.exec().map_err(|source| WorkspaceError::Metadata {
        manifest: manifest.to_path_buf(),
        source,
    })
}

/// Selects a named workspace member and validates its deployment features.
fn resolve_package(
    metadata: &Metadata,
    name: &str,
    selected_features: &[String],
) -> Result<ResolvedPackage, WorkspaceError> {
    let package = workspace_member(metadata, name)?;

    let facets = validate_facets(package)?;
    let mut features = selected_features.to_vec();
    features.sort();
    features.dedup();
    for feature in &features {
        if matches!(feature.as_str(), DESCRIPTOR_FEATURE | PAYLOAD_FEATURE) {
            return Err(WorkspaceError::ReservedFeature {
                package: name.to_owned(),
                feature: feature.clone(),
            });
        }
        if !package.features.contains_key(feature) {
            return Err(WorkspaceError::UnknownFeature {
                package: name.to_owned(),
                feature: feature.clone(),
            });
        }
    }

    Ok(ResolvedPackage {
        id: package.id.clone(),
        name: package.name.to_string(),
        manifest_path: package.manifest_path.clone().into_std_path_buf(),
        features,
        facets,
    })
}

/// Finds a named package only when Cargo reports it as a workspace member.
fn workspace_member<'a>(metadata: &'a Metadata, name: &str) -> Result<&'a Package, WorkspaceError> {
    metadata
        .packages
        .iter()
        .find(|package| package.name == name && metadata.workspace_members.contains(&package.id))
        .ok_or_else(|| {
            if metadata.packages.iter().any(|package| package.name == name) {
                WorkspaceError::NonmemberPackage {
                    package: name.to_owned(),
                }
            } else {
                WorkspaceError::UnknownPackage {
                    package: name.to_owned(),
                }
            }
        })
}

/// Confirms that a package supports both reserved deployment facets.
fn validate_facets(package: &Package) -> Result<FacetFeatures, WorkspaceError> {
    for feature in [DESCRIPTOR_FEATURE, PAYLOAD_FEATURE] {
        if !package.features.contains_key(feature) {
            return Err(WorkspaceError::MissingFacet {
                package: package.name.to_string(),
                feature,
            });
        }
    }
    Ok(FacetFeatures {
        descriptor: DESCRIPTOR_FEATURE,
        payload: PAYLOAD_FEATURE,
    })
}

/// Resolves workspace-relative Federate configuration paths.
fn resolve_federate(workspace_root: &Path, federate: &Federate) -> ResolvedFederate {
    ResolvedFederate {
        groups: federate.groups.clone(),
        target: federate.target.clone(),
        toolchain: federate.toolchain.clone(),
        profile: federate.profile.clone(),
        runtime: federate.runtime.clone(),
        target_json: resolve_optional_path(workspace_root, federate.target_json.as_deref()),
        cargo_config: resolve_optional_path(workspace_root, federate.cargo_config.as_deref()),
    }
}

/// Makes an optional configuration path absolute against the Cargo workspace root.
fn resolve_optional_path(workspace_root: &Path, value: Option<&str>) -> Option<PathBuf> {
    value.map(Path::new).map(|path| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            workspace_root.join(path)
        }
    })
}

/// Reads and fingerprints the exact source lockfile bytes.
fn lockfile_identity(path: PathBuf) -> Result<LockfileIdentity, WorkspaceError> {
    let path = fs::canonicalize(&path).map_err(|source| WorkspaceError::Lockfile {
        path: path.clone(),
        source,
    })?;
    let bytes = fs::read(&path).map_err(|source| WorkspaceError::Lockfile {
        path: path.clone(),
        source,
    })?;
    Ok(LockfileIdentity {
        path,
        digest: *blake3::hash(&bytes).as_bytes(),
    })
}
