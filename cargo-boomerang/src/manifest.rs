use std::{collections::BTreeMap, path::PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Deserializer};

const SUPPORTED_SCHEMA: u32 = 2;

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
    pub fn deployment(&self, name: &str) -> Result<&Deployment> {
        self.deployments
            .get(name)
            .ok_or_else(|| anyhow!("deployment {name} is not defined in Boomerang.toml"))
    }

    fn validate(&self) -> Result<()> {
        if !(1..=SUPPORTED_SCHEMA).contains(&self.schema) {
            bail!(
                "unsupported Boomerang.toml schema {}; expected 1 or {SUPPORTED_SCHEMA}",
                self.schema
            );
        }
        for (name, deployment) in &self.deployments {
            if name.is_empty()
                || matches!(name.as_str(), "." | "..")
                || !name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
            {
                return Err(invalid_deployment(
                    name,
                    "deployment names must be non-empty and contain only ASCII letters, digits, '-', '_', or '.'; '.' and '..' are reserved",
                ));
            }
            deployment.validate(name, self.schema)?;
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

/// One named deployment variant, parameterized by its Federate representation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Deployment<F = Federate> {
    /// Component-instance paths mapped to implementation packages.
    #[serde(default)]
    pub bindings: BTreeMap<String, Binding>,
    /// Federate identifiers mapped to their placement and build configuration.
    pub federates: BTreeMap<String, F>,
    /// Distributed coordination selection for a multi-Federate deployment.
    pub coordination: Option<Coordination>,
    /// Coordinator artifact configuration for the `central-rti` backend.
    pub rti: Option<Rti>,
    /// Deployment-wide execution behavior introduced by schema 2.
    pub execution: Option<ExecutionPolicy>,
}

impl Deployment<Federate> {
    fn validate(&self, name: &str, schema: u32) -> Result<()> {
        if schema == 1 && self.execution.is_some() {
            return Err(invalid_deployment(
                name,
                format!("deployments.{name}.execution is available only in schema 2"),
            ));
        }
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

/// Deployment-wide runtime behavior normalized at the manifest boundary.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct ExecutionPolicy {
    /// Whether logical execution bypasses wall-clock synchronization.
    #[serde(default)]
    pub fast_forward: bool,
    /// Whether schedulers remain alive without pending events.
    #[serde(default)]
    pub keep_alive: bool,
    /// Optional logical horizon normalized once to nonnegative nanoseconds.
    #[serde(default, deserialize_with = "deserialize_logical_horizon")]
    pub logical_horizon: Option<u64>,
}

/// Deserializes an optional human duration as an exact nonnegative nanosecond count.
fn deserialize_logical_horizon<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer)?
        .map(|value| {
            let duration = humantime::parse_duration(&value)
                .map_err(|error| serde::de::Error::custom(error.to_string()))?;
            u64::try_from(duration.as_nanos())
                .map_err(|_| serde::de::Error::custom("logical horizon exceeds u64 nanoseconds"))
        })
        .transpose()
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

/// Parses and manifest-locally validates `Boomerang.toml` source text.
pub fn parse_manifest(source: &str) -> Result<Manifest> {
    let deserializer = toml::Deserializer::parse(source)
        .map_err(|error| invalid_manifest("root", error.to_string()))?;
    let manifest: Manifest = serde_path_to_error::deserialize(deserializer).map_err(|error| {
        let message = error.inner().to_string();
        invalid_manifest(
            &diagnostic_path(&error.path().to_string(), &message),
            message,
        )
    })?;
    manifest.validate()?;
    Ok(manifest)
}

/// Reads, parses, and manifest-locally validates a `Boomerang.toml` file.
pub fn load_manifest(path: impl Into<PathBuf>) -> Result<Manifest> {
    let path = path.into();
    let source = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    parse_manifest(&source)
}

/// Builds a manifest-local deployment diagnostic with its deployment name attached.
fn invalid_deployment(name: &str, message: impl std::fmt::Display) -> anyhow::Error {
    anyhow!("invalid deployment {name}: {message}")
}

/// Builds a user-facing manifest parse diagnostic with its TOML path attached.
fn invalid_manifest(path: &str, message: impl std::fmt::Display) -> anyhow::Error {
    anyhow!("invalid Boomerang.toml at {path}: {message}")
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
