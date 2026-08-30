use std::{collections::BTreeMap, ops::Range, path::PathBuf};

use serde::Deserialize;
use thiserror::Error;

const SUPPORTED_SCHEMA: u32 = 1;

/// A parsed and manifest-locally validated `Boomerang.toml` file.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// Manifest schema version.
    pub schema: u32,
    /// Host-compatible package and entry point that declare application topology.
    pub topology: Topology,
    /// Named deployment variants.
    pub deployments: BTreeMap<String, Deployment>,
}

impl Manifest {
    /// Returns a named deployment or a diagnostic identifying the missing name.
    pub fn deployment(&self, name: &str) -> Result<&Deployment, ManifestError> {
        self.deployments
            .get(name)
            .ok_or_else(|| ManifestError::UnknownDeployment {
                name: name.to_owned(),
            })
    }

    fn validate(&self) -> Result<(), ManifestError> {
        if self.schema != SUPPORTED_SCHEMA {
            return Err(ManifestError::UnsupportedSchema { found: self.schema });
        }
        for (name, deployment) in &self.deployments {
            deployment.validate(name)?;
        }
        Ok(())
    }
}

/// The package and exported entry point that declare application topology.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Topology {
    /// Cargo package containing the topology declaration.
    pub package: String,
    /// Rust path to the topology entry point.
    pub entry: String,
}

/// One named deployment variant.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Deployment {
    /// Component-instance paths mapped to implementation packages.
    #[serde(default)]
    pub bindings: BTreeMap<String, Binding>,
    /// Federate identifiers mapped to their placement and build configuration.
    pub federates: BTreeMap<String, Federate>,
    /// Distributed coordination selection for a multi-Federate deployment.
    pub coordination: Option<Coordination>,
    /// Coordinator artifact configuration for the `central-rti` backend.
    pub rti: Option<Rti>,
}

impl Deployment {
    fn validate(&self, name: &str) -> Result<(), ManifestError> {
        match self.federates.len() {
            0 => {
                return Err(invalid_deployment(
                    name,
                    format!("deployments.{name}.federates must contain at least one Federate"),
                ))
            }
            1 if self.coordination.is_some() => {
                return Err(invalid_deployment(
                    name,
                    "coordination is absent for one-Federate deployments",
                ))
            }
            count if count > 1 && self.coordination.is_none() => {
                return Err(invalid_deployment(
                    name,
                    format!(
                        "deployments.{name}.coordination is required for multi-Federate deployments"
                    ),
                ))
            }
            _ => {}
        }

        match (
            self.coordination.as_ref().map(|value| value.backend),
            &self.rti,
        ) {
            (Some(CoordinationBackend::CentralRti), None) => Err(invalid_deployment(
                name,
                format!("central-rti requires deployments.{name}.rti"),
            )),
            (Some(CoordinationBackend::PeerToPeer), Some(_)) => Err(invalid_deployment(
                name,
                format!("deployments.{name}.rti is not valid with peer-to-peer"),
            )),
            (None, Some(_)) => Err(invalid_deployment(
                name,
                format!("deployments.{name}.rti is valid only with central-rti"),
            )),
            _ => Ok(()),
        }
    }
}

/// Selection of one implementation package for a component instance.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    /// Cargo package providing the selected implementation descriptor.
    pub package: String,
    /// Cargo features enabled while compiling that implementation descriptor.
    #[serde(default)]
    pub features: Vec<String>,
}

/// Placement and Cargo build configuration for one Federate.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Federate {
    /// Stable placement groups assigned to the Federate.
    pub groups: Vec<String>,
    /// Optional Rust target triple; absence selects the host target.
    pub target: Option<String>,
    /// Optional Rust toolchain selector.
    pub toolchain: Option<String>,
    /// Optional Cargo profile; absence selects Cargo's development profile.
    pub profile: Option<String>,
    /// Runtime backend required by the generated Federate.
    pub runtime: String,
    /// Optional path to a custom target JSON file.
    pub target_json: Option<String>,
    /// Optional Cargo configuration file used for this Federate invocation.
    pub cargo_config: Option<String>,
}

/// Distributed coordination configuration for a deployment.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Coordination {
    /// Selected coordination backend.
    pub backend: CoordinationBackend,
}

/// Coordination backends reserved by the deployment manifest schema.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum CoordinationBackend {
    /// Federates coordinate through a generated central RTI artifact.
    CentralRti,
    /// Reserved RTI-free peer coordination; compilation support is deferred.
    PeerToPeer,
}

/// Cargo build configuration for the central RTI artifact.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Rti {
    /// Rust target triple for the coordinator artifact.
    pub target: String,
    /// Optional Cargo profile; absence selects Cargo's development profile.
    pub profile: Option<String>,
}

/// Failure while reading, parsing, or manifest-locally validating `Boomerang.toml`.
#[derive(Debug, Error)]
pub enum ManifestError {
    /// The manifest file could not be read.
    #[error("failed to read {path}: {source}")]
    Read {
        /// Filesystem path that could not be read.
        path: PathBuf,
        /// Underlying filesystem error.
        #[source]
        source: std::io::Error,
    },
    /// TOML syntax or schema deserialization failed.
    #[error("invalid Boomerang.toml at {path}: {message}")]
    Parse {
        /// TOML path at which deserialization failed.
        path: String,
        /// Parser or schema diagnostic.
        message: String,
        /// Byte range reported by the TOML parser when available.
        span: Option<Range<usize>>,
    },
    /// The manifest declares a schema version this tool does not understand.
    #[error("unsupported Boomerang.toml schema {found}; expected {SUPPORTED_SCHEMA}")]
    UnsupportedSchema {
        /// Unsupported schema version found in the manifest.
        found: u32,
    },
    /// A named deployment violates manifest-local consistency rules.
    #[error("invalid deployment {deployment}: {message}")]
    InvalidDeployment {
        /// Name of the invalid deployment variant.
        deployment: String,
        /// Manifest-local consistency diagnostic.
        message: String,
    },
    /// A caller requested a deployment name absent from the manifest.
    #[error("deployment {name} is not defined in Boomerang.toml")]
    UnknownDeployment {
        /// Requested deployment name.
        name: String,
    },
}

/// Parses and manifest-locally validates `Boomerang.toml` source text.
pub fn parse_manifest(source: &str) -> Result<Manifest, ManifestError> {
    let deserializer = toml::Deserializer::parse(source).map_err(|error| ManifestError::Parse {
        path: String::from("root"),
        span: error.span(),
        message: error.to_string(),
    })?;
    let manifest: Manifest = serde_path_to_error::deserialize(deserializer).map_err(|error| {
        let message = error.inner().to_string();
        ManifestError::Parse {
            path: diagnostic_path(&error.path().to_string(), &message),
            span: error.inner().span(),
            message,
        }
    })?;
    manifest.validate()?;
    Ok(manifest)
}

/// Reads, parses, and manifest-locally validates a `Boomerang.toml` file.
pub fn load_manifest(path: impl Into<PathBuf>) -> Result<Manifest, ManifestError> {
    let path = path.into();
    let source = std::fs::read_to_string(&path).map_err(|source| ManifestError::Read {
        path: path.clone(),
        source,
    })?;
    parse_manifest(&source)
}

/// Builds a manifest-local deployment diagnostic with its deployment name attached.
fn invalid_deployment(name: &str, message: impl Into<String>) -> ManifestError {
    ManifestError::InvalidDeployment {
        deployment: name.to_owned(),
        message: message.into(),
    }
}

/// Extends Serde's containing-table path with an unknown field when TOML reports one.
fn diagnostic_path(path: &str, message: &str) -> String {
    let mut path = match path {
        "." | "" => String::from("root"),
        value => value.to_owned(),
    };
    if let Some(field) = unknown_field(message) {
        if path == "root" {
            path = field.to_owned();
        } else if !path.ends_with(field) {
            path.push('.');
            path.push_str(field);
        }
    }
    path
}

/// Extracts the field name from Serde's stable unknown-field diagnostic shape.
fn unknown_field(message: &str) -> Option<&str> {
    let field = message.split_once("unknown field `")?.1;
    field.split_once('`').map(|(field, _)| field)
}
