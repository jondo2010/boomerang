//! Complete static-launcher build and fingerprinted bundle publication pipeline.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;

use crate::{
    bundle::{
        publish_bundle, BindingDocument, BundleSource, CoordinationDocument, DeploymentDocument,
        DescriptorDocument, ExecutionPolicyDocument, FederateDocument, PackageDocument,
        DEPLOYMENT_SCHEMA,
    },
    check::{analyze, resource_report, ResourceReport, COMPILER_SCHEMA},
    codegen::generate_analyzed_launcher,
};

/// Stable domain separator for schema-v2 deployment fingerprint inputs.
const DEPLOYMENT_FINGERPRINT_DOMAIN_V2: &str = "boomerang.deployment.v2";

/// Canonical semantic input for schema-v2 deployment fingerprints.
#[derive(Serialize)]
struct FingerprintInputV2 {
    /// Stable domain separator for schema-v2 deployment fingerprints.
    domain: &'static str,
    /// Deployment-document schema version.
    schema: u32,
    /// Canonical compiler-image schema version.
    compiler_schema: u32,
    /// Lowercase BLAKE3 hash of compact canonical topology JSON.
    topology_hash: String,
    /// Selected implementation bindings in canonical driver order.
    bindings: Vec<BindingDocument>,
    /// Lowercase BLAKE3 hash of the source workspace lockfile.
    source_lock_hash: String,
    /// Lowercase BLAKE3 hash of the reconciled generated lockfile.
    generated_lock_hash: String,
    /// Lowercase BLAKE3 hash of generated Rust launcher source.
    generated_source_hash: String,
    /// Federate target and runtime selections in compiler identity order.
    federates: Vec<FederateDocument>,
    /// Deployment execution policy embedded in generated source.
    execution: ExecutionPolicyDocument,
    /// Canonical static resource projection.
    resources: ResourceReport,
    /// Selected coordination backend and protocol identity.
    coordination: CoordinationDocument,
}

/// Builds one deployment and returns its immutable `deployment.json` path.
pub fn build(workspace: impl AsRef<Path>, deployment_name: &str) -> Result<PathBuf> {
    let analyzed = analyze(workspace, deployment_name)?;
    let compiled_federates = analyzed.compiled.federates();
    if compiled_federates.len() != 1 {
        bail!("deployment bundle generation currently supports one local Federate");
    }
    let compiled_federate = &compiled_federates[0];
    let federate_id = compiled_federate.id().as_str();
    let configuration = analyzed
        .resolved
        .deployment()
        .federates
        .get(federate_id)
        .ok_or_else(|| anyhow!("deployment has no configuration for Federate '{federate_id}'"))?;
    let target_json_hash = optional_hash(configuration.target_json.as_deref())
        .context("failed to hash configured target JSON before launcher generation")?;
    let cargo_config_hash = optional_hash(configuration.cargo_config.as_deref())
        .context("failed to hash configured Cargo configuration before launcher generation")?;

    let generated = generate_analyzed_launcher(&analyzed, federate_id).with_context(|| {
        format!(
            "deployment '{deployment_name}' Federate '{federate_id}' launcher generation failed"
        )
    })?;
    let built = generated.build_locked_offline().with_context(|| {
        format!("deployment '{deployment_name}' Federate '{federate_id}' launcher build failed")
    })?;
    verify_unchanged_hash(
        "configured target JSON",
        configuration.target_json.as_deref(),
        target_json_hash.as_deref(),
    )?;
    verify_unchanged_hash(
        "configured Cargo configuration",
        configuration.cargo_config.as_deref(),
        cargo_config_hash.as_deref(),
    )?;

    let topology = serde_json::to_vec(analyzed.driver.topology())
        .context("failed to serialize canonical topology")?;
    let topology_hash = hash_bytes(&topology);
    let bindings = binding_records(&analyzed)?;
    let mut groups = configuration.groups.clone();
    groups.sort();
    groups.dedup();
    let federate = FederateDocument {
        id: federate_id.to_owned(),
        groups,
        target: compiled_federate.target().to_string(),
        toolchain: configuration.toolchain.clone(),
        profile: configuration.profile.clone(),
        runtime: compiled_federate.runtime().to_string(),
        target_json_hash,
        cargo_config_hash,
    };
    let resources = resource_report(&analyzed.compiled);
    let source_lock_hash = lowercase_hex(&analyzed.resolved.lockfile().digest);
    let generated_lock_hash = hash_file(generated.lockfile_path())?;
    let generated_source_hash = hash_file(generated.source_path())?;
    let execution = analyzed
        .resolved
        .deployment()
        .execution
        .clone()
        .unwrap_or_default();
    let execution = ExecutionPolicyDocument {
        fast_forward: execution.fast_forward,
        keep_alive: execution.keep_alive,
        logical_horizon_nanos: execution.logical_horizon,
    };
    let coordination = CoordinationDocument {
        backend: String::from("local"),
        protocol: None,
    };
    let fingerprint_input = FingerprintInputV2 {
        domain: DEPLOYMENT_FINGERPRINT_DOMAIN_V2,
        schema: DEPLOYMENT_SCHEMA,
        compiler_schema: COMPILER_SCHEMA,
        topology_hash: topology_hash.clone(),
        bindings,
        source_lock_hash,
        generated_lock_hash,
        generated_source_hash,
        federates: vec![federate],
        execution,
        resources: resources.clone(),
        coordination,
    };
    let fingerprint_bytes = serde_json::to_vec(&fingerprint_input)
        .context("failed to serialize canonical deployment fingerprint input")?;
    let fingerprint = hash_bytes(&fingerprint_bytes);

    let document = DeploymentDocument {
        schema: DEPLOYMENT_SCHEMA,
        compiler_schema: COMPILER_SCHEMA,
        deployment: deployment_name.to_owned(),
        fingerprint,
        topology_hash,
        source_lock_hash: fingerprint_input.source_lock_hash,
        generated_lock_hash: fingerprint_input.generated_lock_hash,
        generated_source_hash: fingerprint_input.generated_source_hash,
        bindings: fingerprint_input.bindings,
        federates: fingerprint_input.federates,
        execution: fingerprint_input.execution,
        resources,
        coordination: fingerprint_input.coordination,
        generated: Vec::new(),
        artifacts: Vec::new(),
    };
    publish_bundle(
        analyzed.resolved.target_directory(),
        document,
        BundleSource {
            federate: federate_id,
            manifest: generated.manifest_path(),
            lockfile: generated.lockfile_path(),
            source: generated.source_path(),
            executable: built.executable_path(),
        },
    )
}

/// Builds canonical fingerprint and document records for selected bindings.
fn binding_records(analyzed: &crate::check::AnalyzedDeployment) -> Result<Vec<BindingDocument>> {
    let mut records = Vec::new();
    for binding in analyzed.driver.bindings() {
        let component = binding.component().to_string();
        let implementation = binding.implementation().as_str();
        let selection = analyzed
            .resolved
            .deployment()
            .bindings
            .get(&component)
            .ok_or_else(|| {
                anyhow!("descriptor driver returned unselected component '{component}'")
            })?;
        let package = analyzed.resolved.package(implementation).ok_or_else(|| {
            anyhow!("descriptor driver returned unresolved package '{implementation}'")
        })?;
        let descriptor = binding.descriptor();
        let descriptor_hash = lowercase_hex(
            &descriptor
                .descriptor_fingerprint_input()
                .fingerprint()
                .to_bytes(),
        );
        let package = PackageDocument {
            name: package.name.clone(),
            version: package.version.clone(),
            source: package.source.clone(),
            features: selection.features.clone(),
        };
        let descriptor = DescriptorDocument {
            component: component.clone(),
            package: implementation.to_owned(),
            contract: descriptor.contract_id().as_str().to_owned(),
            contract_version: descriptor.contract_version(),
            fingerprint: descriptor_hash,
            macro_abi: descriptor.macro_abi(),
        };
        records.push(BindingDocument {
            component,
            package,
            descriptor,
        });
    }
    Ok(records)
}

/// Hashes an optional exact configuration file without embedding its path.
fn optional_hash(path: Option<&Path>) -> Result<Option<String>> {
    path.map(hash_file).transpose()
}

/// Verifies that a configuration file still has its pre-build content hash.
fn verify_unchanged_hash(
    description: &str,
    path: Option<&Path>,
    expected: Option<&str>,
) -> Result<()> {
    let actual = optional_hash(path)
        .with_context(|| format!("failed to re-hash {description} after launcher build"))?;
    if actual.as_deref() != expected {
        bail!("{description} changed while building deployment");
    }
    Ok(())
}

/// Hashes exact file bytes as lowercase BLAKE3 text.
fn hash_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    Ok(hash_bytes(&bytes))
}

/// Hashes exact bytes as lowercase BLAKE3 text.
fn hash_bytes(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

/// Encodes bytes as canonical lowercase hexadecimal text.
fn lowercase_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(output, "{byte:02x}").expect("writing into a String cannot fail");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn post_build_configuration_hash_check_rejects_changed_file() {
        let directory = tempfile::tempdir().unwrap();
        let target_json = directory.path().join("target.json");
        fs::write(&target_json, b"before").unwrap();
        let expected = optional_hash(Some(&target_json)).unwrap();

        fs::write(&target_json, b"after").unwrap();
        let error = verify_unchanged_hash(
            "configured target JSON",
            Some(&target_json),
            expected.as_deref(),
        )
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("configured target JSON changed while building deployment"));
    }

    #[test]
    fn fingerprint_input_serializes_the_v2_deployment_domain_first() {
        let input = FingerprintInputV2 {
            domain: DEPLOYMENT_FINGERPRINT_DOMAIN_V2,
            schema: 2,
            compiler_schema: 1,
            topology_hash: String::new(),
            bindings: Vec::new(),
            source_lock_hash: String::new(),
            generated_lock_hash: String::new(),
            generated_source_hash: String::new(),
            federates: Vec::new(),
            execution: serde_json::from_value(serde_json::json!({
                "fast_forward": false,
                "keep_alive": false,
                "logical_horizon_nanos": null
            }))
            .unwrap(),
            resources: serde_json::from_value(serde_json::json!({ "federates": [] })).unwrap(),
            coordination: serde_json::from_value(serde_json::json!({
                "backend": "local",
                "protocol": null
            }))
            .unwrap(),
        };

        assert!(serde_json::to_string(&input)
            .unwrap()
            .starts_with(r#"{"domain":"boomerang.deployment.v2","schema":2"#));
    }
}
