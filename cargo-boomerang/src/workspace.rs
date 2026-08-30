use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use cargo_metadata::{Metadata, MetadataCommand, Package, PackageId};
use thiserror::Error;

use crate::{load_manifest, Deployment, Federate, ManifestError, Topology};

const DESCRIPTOR_FEATURE: &str = "__boomerang_descriptor";
const PAYLOAD_FEATURE: &str = "__boomerang_payload";

/// Exact Cargo identity and location for a selected workspace package.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CargoPackage {
    /// Cargo package name used in generated dependency declarations.
    pub name: String,
    /// Exact package version reported by Cargo metadata.
    pub version: String,
    /// Cargo source identity, or `None` for a local path package.
    pub source: Option<String>,
    /// Rust library target name when the package exposes one.
    pub lib_target: Option<String>,
    /// Opaque package identity reported by Cargo.
    pub id: PackageId,
    /// Absolute path to the selected package manifest.
    pub manifest_path: PathBuf,
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
    /// Cargo target directory that owns generated deployment tooling.
    target_directory: PathBuf,
    /// Manifest selection of the topology package and entry point.
    topology: Topology,
    /// Name of the selected deployment variant.
    deployment_name: String,
    /// Selected deployment with normalized bindings and resolved Federate paths.
    deployment: Deployment<ResolvedFederate>,
    /// Selected package names mapped to exact Cargo identities and locations.
    packages: BTreeMap<String, CargoPackage>,
    /// Exact host compiler package used by the generated descriptor driver.
    host_builder: CargoPackage,
    /// Exact package identities available in the source application's lock graph.
    locked_package_ids: BTreeSet<String>,
    /// Identity of the lockfile enforced during Cargo resolution.
    lockfile: LockfileIdentity,
}

impl ResolvedWorkspace {
    /// Returns Cargo's application target directory.
    pub fn target_directory(&self) -> &Path {
        &self.target_directory
    }

    /// Returns the selected topology package and entry point.
    pub fn topology(&self) -> &Topology {
        &self.topology
    }

    /// Returns the name of the selected deployment variant.
    pub fn deployment_name(&self) -> &str {
        &self.deployment_name
    }

    /// Returns the selected deployment with resolved Federate paths.
    pub fn deployment(&self) -> &Deployment<ResolvedFederate> {
        &self.deployment
    }

    /// Returns the exact Cargo identity and location for a selected package.
    pub fn package(&self, name: &str) -> Option<&CargoPackage> {
        self.packages.get(name)
    }

    /// Returns the exact host compiler package selected by Cargo metadata.
    pub fn host_builder(&self) -> &CargoPackage {
        &self.host_builder
    }

    /// Returns all direct packages expected beneath the synthetic driver root.
    pub(crate) fn driver_package_ids(&self) -> BTreeSet<String> {
        self.packages
            .values()
            .chain(std::iter::once(&self.host_builder))
            .map(|package| package.id.to_string())
            .collect()
    }

    /// Returns every exact package identity available to the source application.
    pub(crate) fn locked_package_ids(&self) -> &BTreeSet<String> {
        &self.locked_package_ids
    }

    /// Returns the identity of the lockfile enforced during resolution.
    pub fn lockfile(&self) -> &LockfileIdentity {
        &self.lockfile
    }
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
    /// Cargo metadata did not contain exactly one host compiler package.
    #[error("expected exactly one boomerang_builder package in locked metadata, found {count}")]
    HostBuilder {
        /// Number of matching package identities reported by Cargo.
        count: usize,
    },
    /// Cargo metadata unexpectedly omitted its resolved dependency graph.
    #[error("locked Cargo metadata did not contain a dependency resolve graph")]
    MissingResolve,
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
    let target_directory = metadata.target_directory.clone().into_std_path_buf();

    let topology_package = resolve_topology(&metadata, &manifest.topology.package)?;
    let mut packages = BTreeMap::from([(manifest.topology.package.clone(), topology_package)]);
    let mut bindings = deployment.bindings.clone();
    for (component, binding) in &mut bindings {
        binding.features.sort();
        binding.features.dedup();
        let package =
            resolve_package(&metadata, &binding.package, &binding.features).map_err(|source| {
                WorkspaceError::Binding {
                    deployment: deployment_name.to_owned(),
                    component: component.clone(),
                    source: Box::new(source),
                }
            })?;
        packages.insert(binding.package.clone(), package);
    }
    let host_builder =
        resolve_host_builder(&metadata, packages.values().map(|package| &package.id))?;
    let locked_package_ids = metadata
        .packages
        .iter()
        .map(|package| package.id.to_string())
        .collect();
    let federates = deployment
        .federates
        .iter()
        .map(|(name, federate)| (name.clone(), resolve_federate(&workspace_root, federate)))
        .collect();
    let lockfile = lockfile_identity(workspace_root.join("Cargo.lock"))?;

    Ok(ResolvedWorkspace {
        target_directory,
        topology: manifest.topology.clone(),
        deployment_name: deployment_name.to_owned(),
        deployment: Deployment {
            bindings,
            federates,
            coordination: deployment.coordination.clone(),
            rti: deployment.rti.clone(),
        },
        packages,
        host_builder,
        locked_package_ids,
        lockfile,
    })
}

/// Resolves the host-compatible topology package without imposing implementation facets.
fn resolve_topology(metadata: &Metadata, name: &str) -> Result<CargoPackage, WorkspaceError> {
    let package = workspace_member(metadata, name)?;
    Ok(cargo_package(package))
}

/// Finds the unique host compiler package already selected by locked application metadata.
fn resolve_host_builder<'a>(
    metadata: &Metadata,
    roots: impl IntoIterator<Item = &'a PackageId>,
) -> Result<CargoPackage, WorkspaceError> {
    let resolve = metadata
        .resolve
        .as_ref()
        .ok_or(WorkspaceError::MissingResolve)?;
    let nodes = resolve
        .nodes
        .iter()
        .map(|node| (&node.id, node))
        .collect::<BTreeMap<_, _>>();
    let packages = metadata
        .packages
        .iter()
        .map(|package| (&package.id, package))
        .collect::<BTreeMap<_, _>>();
    let mut pending = roots.into_iter().cloned().collect::<Vec<_>>();
    let mut visited = BTreeSet::new();
    let mut builders = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !visited.insert(id.clone()) {
            continue;
        }
        if packages
            .get(&id)
            .is_some_and(|package| package.name == "boomerang_builder")
        {
            builders.insert(id.clone());
        }
        if let Some(node) = nodes.get(&id) {
            pending.extend(node.deps.iter().map(|dependency| dependency.pkg.clone()));
        }
    }
    match builders.into_iter().collect::<Vec<_>>().as_slice() {
        [id] => Ok(cargo_package(packages[id])),
        values => Err(WorkspaceError::HostBuilder {
            count: values.len(),
        }),
    }
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
) -> Result<CargoPackage, WorkspaceError> {
    let package = workspace_member(metadata, name)?;

    validate_facets(package)?;
    for feature in selected_features {
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

    Ok(cargo_package(package))
}

/// Copies the Cargo identity fields required by generated dependency declarations.
fn cargo_package(package: &Package) -> CargoPackage {
    CargoPackage {
        name: package.name.to_string(),
        version: package.version.to_string(),
        source: package.source.as_ref().map(ToString::to_string),
        lib_target: package
            .targets
            .iter()
            .find(|target| {
                target
                    .kind
                    .iter()
                    .any(|kind| matches!(kind, cargo_metadata::TargetKind::Lib))
            })
            .map(|target| target.name.clone()),
        id: package.id.clone(),
        manifest_path: package.manifest_path.clone().into_std_path_buf(),
    }
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
fn validate_facets(package: &Package) -> Result<(), WorkspaceError> {
    for feature in [DESCRIPTOR_FEATURE, PAYLOAD_FEATURE] {
        if !package.features.contains_key(feature) {
            return Err(WorkspaceError::MissingFacet {
                package: package.name.to_string(),
                feature,
            });
        }
    }
    Ok(())
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
