//! Complete static-launcher build and fingerprinted bundle publication pipeline.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, bail, Context, Result};

use crate::{
    bundle::{
        deployment_fingerprint, publish_bundle, BindingDocument, BundleSource,
        CoordinationDocument, DeploymentDocument, DescriptorDocument, ExecutionPolicyDocument,
        FederateDocument, PackageDocument, DEPLOYMENT_SCHEMA,
    },
    check::{analyze, resource_report, AnalyzedDeployment, COMPILER_SCHEMA},
    codegen::generate_analyzed_launcher,
};

/// Builds one deployment and returns its immutable `deployment.json` path.
pub fn build(workspace: impl AsRef<Path>, deployment_name: &str) -> Result<PathBuf> {
    let analyzed = analyze(workspace, deployment_name)?;
    build_analyzed(&analyzed)
}

/// Builds and publishes a deployment from an already validated analysis.
pub(crate) fn build_analyzed(analyzed: &AnalyzedDeployment) -> Result<PathBuf> {
    let deployment_name = analyzed.resolved.deployment_name();
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
    let mut document = DeploymentDocument {
        schema: DEPLOYMENT_SCHEMA,
        compiler_schema: COMPILER_SCHEMA,
        deployment: deployment_name.to_owned(),
        fingerprint: String::new(),
        topology_hash,
        source_lock_hash,
        generated_lock_hash,
        generated_source_hash,
        bindings,
        federates: vec![federate],
        execution,
        resources,
        coordination,
        generated: Vec::new(),
        artifacts: Vec::new(),
    };
    document.fingerprint = deployment_fingerprint(&document)?;
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
}
