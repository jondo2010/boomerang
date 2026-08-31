//! Complete host-side deployment checking and report publication.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use boomerang_builder::compiler::{
    lower, CoordinationBackendId, CoordinationSelection, FederateConfig, FederateId,
    ImplementationBinding, OwnedCompiledDeployment, PlacementAssignment, PlacementGroupId,
    ResolvedDeployment, RuntimeBackendId, TargetTriple,
};
use serde::Serialize;

use crate::{
    driver::{run_resolved_descriptor_driver, DriverOutput},
    resolve_workspace, CoordinationBackend, ResolvedWorkspace,
};

const COMPILER_SCHEMA: u32 = 1;

/// Runs the complete host-side check pipeline and returns the published report path.
pub fn check(workspace: impl AsRef<Path>, deployment_name: &str) -> Result<PathBuf> {
    let resolved = resolve_workspace(workspace, deployment_name)?;
    let driver = run_resolved_descriptor_driver(&resolved)?;
    let deployment = build_resolved_deployment(&resolved, &driver)?;
    let compiled = lower(&deployment).context("failed to lower resolved deployment")?;
    compiled
        .validate()
        .context("failed to validate compiled deployment")?;
    let report = build_report(deployment_name, driver.topology(), &compiled)?;
    publish_report(&resolved, &report)
}

/// Converts manifest and descriptor-driver selections into the canonical compiler input.
fn build_resolved_deployment(
    resolved: &ResolvedWorkspace,
    driver: &DriverOutput,
) -> Result<ResolvedDeployment> {
    let bindings = driver.bindings().iter().map(|binding| {
        ImplementationBinding::new(
            binding.component().clone(),
            binding.implementation().clone(),
            binding.descriptor().clone(),
        )
    });
    let placements = resolved
        .deployment()
        .federates
        .iter()
        .flat_map(|(federate, config)| {
            config.groups.iter().map(move |group| {
                Ok(PlacementAssignment::new(
                    PlacementGroupId::new(group)?,
                    FederateId::new(federate.as_str())?,
                ))
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let federates = resolved
        .deployment()
        .federates
        .iter()
        .map(|(id, config)| {
            let target = config
                .target
                .clone()
                .unwrap_or_else(|| target_lexicon::HOST.to_string());
            Ok(FederateConfig::new(
                FederateId::new(id.as_str())?,
                TargetTriple::new(target)?,
                RuntimeBackendId::new(config.runtime.as_str())?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    let coordination = match resolved.deployment().coordination.as_ref() {
        None => CoordinationSelection::Local,
        Some(coordination) => CoordinationSelection::Distributed {
            backend: CoordinationBackendId::new(match coordination.backend {
                CoordinationBackend::CentralRti => "central-rti",
                CoordinationBackend::PeerToPeer => "peer-to-peer",
            })?,
        },
    };

    ResolvedDeployment::new(
        driver.topology().clone(),
        bindings,
        placements,
        federates,
        coordination,
        [],
    )
    .context("failed to resolve deployment selections")
}

/// Versioned, deterministic result of a successful deployment check.
#[derive(Serialize)]
struct CheckReport<'a> {
    /// Schema version for the compiler report document.
    compiler_schema: u32,
    /// Selected deployment name.
    deployment: &'a str,
    /// BLAKE3 digest of the validated target-neutral topology.
    topology_digest: String,
    /// Canonical Federate and Enclave resource bounds.
    resources: ResourceReport,
    /// Structured diagnostics emitted by successful analysis.
    diagnostics: Vec<CheckDiagnostic>,
}

/// Canonically ordered resource bounds for the checked deployment.
#[derive(Serialize)]
struct ResourceReport {
    /// Federates in compiler identity order.
    federates: Vec<FederateResourceReport>,
}

/// Resource projection for one compiled Federate.
#[derive(Serialize)]
struct FederateResourceReport {
    /// Stable Federate identity.
    id: String,
    /// Selected Rust compilation target.
    target: String,
    /// Selected runtime backend.
    runtime: String,
    /// Enclaves owned by this Federate in compiler identity order.
    enclaves: Vec<EnclaveResourceReport>,
}

/// Fixed storage bounds for one compiled Enclave.
#[derive(Serialize)]
struct EnclaveResourceReport {
    /// Stable Enclave identity.
    id: String,
    /// Maximum number of reactor-state slots.
    state_slots: u32,
    /// Maximum number of logical-action slots.
    action_slots: u32,
    /// Maximum number of queued runtime events.
    event_capacity: u32,
    /// Maximum payload storage in bytes.
    payload_bytes: u64,
    /// Maximum reactor-state storage in bytes.
    state_bytes: u64,
    /// Maximum scheduler scratch storage in bytes.
    scratch_bytes: u64,
}

/// One structured successful-analysis diagnostic reserved by report schema 1.
#[derive(Serialize)]
struct CheckDiagnostic {
    /// Stable machine-readable diagnostic code.
    code: String,
    /// Human-readable diagnostic text.
    message: String,
}

/// Projects validated compiler output into its stable JSON report schema.
fn build_report<'a>(
    deployment_name: &'a str,
    topology: &boomerang_builder::compiler::ApplicationTopology,
    compiled: &OwnedCompiledDeployment,
) -> Result<CheckReport<'a>> {
    let topology = serde_json::to_vec(topology).context("failed to serialize topology")?;
    let topology_digest = format!("blake3:{}", blake3::hash(&topology).to_hex());
    let federates = compiled
        .federates()
        .iter()
        .map(|federate| FederateResourceReport {
            id: federate.id().to_string(),
            target: federate.target().to_string(),
            runtime: federate.runtime().to_string(),
            enclaves: federate
                .enclaves()
                .iter()
                .map(|enclave| {
                    let bounds = enclave.image().storage_bounds;
                    EnclaveResourceReport {
                        id: enclave.id().to_string(),
                        state_slots: bounds.state_slots(),
                        action_slots: bounds.action_slots(),
                        event_capacity: bounds.event_capacity(),
                        payload_bytes: bounds.payload_bytes(),
                        state_bytes: bounds.state_bytes(),
                        scratch_bytes: bounds.scratch_bytes(),
                    }
                })
                .collect(),
        })
        .collect();
    Ok(CheckReport {
        compiler_schema: COMPILER_SCHEMA,
        deployment: deployment_name,
        topology_digest,
        resources: ResourceReport { federates },
        diagnostics: Vec::new(),
    })
}

/// Atomically publishes one successful report beside any previous valid report.
fn publish_report(resolved: &ResolvedWorkspace, report: &CheckReport<'_>) -> Result<PathBuf> {
    let directory = resolved
        .target_directory()
        .join("boomerang")
        .join(resolved.deployment_name());
    fs::create_dir_all(&directory)
        .with_context(|| format!("failed to prepare {}", directory.display()))?;
    let path = directory.join("check.json");
    let mut temporary = tempfile::NamedTempFile::new_in(&directory)
        .with_context(|| format!("failed to prepare {}", directory.display()))?;
    serde_json::to_writer_pretty(&mut temporary, report)
        .with_context(|| format!("failed to write {}", path.display()))?;
    temporary.write_all(b"\n")?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(&path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to publish {}", path.display()))?;
    Ok(path)
}
