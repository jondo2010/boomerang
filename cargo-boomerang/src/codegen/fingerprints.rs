//! Compiler-side inputs for channel compatibility and local image identity.
//!
//! Existing descriptor fingerprints establish descriptor/payload ABI compatibility;
//! bundle fingerprints identify reproducible deployment inputs; artifact digests identify
//! exact executable bytes. None can admit heterogeneous Federates: changing one private
//! image or binary must not invalidate otherwise compatible channel peers.
//!
//! This module reuses BLAKE3 with separate versioned domains for the dense mapping,
//! shared coordination semantics, and each local scheduler image. It reuses existing
//! descriptor fingerprints and the canonical image renderer as inputs. Only shared
//! coordination and mapping digests participate in channel admission. Declared contract
//! IDs and versions own payload schemas; local image and artifact claims may differ.

use super::{binding_implementation, rust};
use anyhow::{Context, Result};
use boomerang_builder::{compiler::FederateSlice, host_interchange::DescriptorDriverBinding};

/// Hashes an explicitly ordered semantic record with an independent versioned domain.
fn hash(domain: &str, input: &impl serde::Serialize) -> Result<blake3::Hash> {
    let bytes = serde_json::to_vec(input).context("failed to encode canonical identity input")?;
    let mut hasher = blake3::Hasher::new_derive_key(domain);
    hasher.update(&bytes);
    Ok(hasher.finalize())
}

pub(super) use boomerang_codegen::coordination::{
    coordination_fingerprint as coordination, mapping_fingerprint as mapping,
};

/// Identifies one actual scheduler image, its direct bindings, descriptors, and storage bounds.
/// The existing canonical image renderer owns the complete scheduler-table encoding.
pub(super) fn federate_image(
    slice: &FederateSlice<'_>,
    bindings: &[DescriptorDriverBinding],
) -> Result<blake3::Hash> {
    let required = slice
        .enclaves()
        .iter()
        .flat_map(|e| e.required_bindings().iter())
        .collect::<Vec<_>>();
    let mut descriptors = std::collections::BTreeMap::new();
    for binding in bindings {
        use boomerang_builder::compiler::RequiredBinding;
        if required.iter().any(|r| {
            let (RequiredBinding::State { component, .. }
            | RequiredBinding::Reaction { component, .. }
            | RequiredBinding::Port { component, .. }
            | RequiredBinding::Action { component, .. }) = r;
            component == binding.component()
        }) {
            descriptors.insert(
                (
                    binding.component().to_string(),
                    binding.implementation().as_str(),
                ),
                binding.descriptor().fingerprint().to_bytes(),
            );
        }
    }
    let bindings = required
        .iter()
        .map(|r| (binding_implementation(r), r.symbol()))
        .collect::<Vec<_>>();
    hash(
        "boomerang.federate-image.v1",
        &(
            rust::image_fingerprint_input(slice)?,
            bindings,
            descriptors.into_iter().collect::<Vec<_>>(),
        ),
    )
}

#[cfg(test)]
mod tests;

/// Includes the selected clock domain in image identity.
/// Without an external clock, preserves the original image fingerprint.
pub(super) fn with_physical_clock(
    image: blake3::Hash,
    clock: Option<&crate::manifest::ExternalClock>,
) -> Result<blake3::Hash> {
    match clock {
        None => Ok(image),
        Some(clock) => hash("boomerang.physical-clock.v1", &(image.as_bytes(), clock)),
    }
}

/// Includes independent input policy in execution artifact identity.
pub(super) fn with_physical_inputs(
    image: blake3::Hash,
    inputs: Option<&crate::manifest::PhysicalInputs>,
) -> Result<blake3::Hash> {
    match inputs {
        None => Ok(image),
        Some(inputs) => hash("boomerang.physical-inputs.v1", &(image.as_bytes(), inputs)),
    }
}
