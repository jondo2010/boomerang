//! Validated, versioned JSON exchanged with generated host descriptor drivers.

use std::{collections::BTreeSet, io};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    compiler::{ApplicationTopology, ComponentInstanceId, ImplementationId, InvalidStableId},
    descriptor::ComponentDescriptor,
};

/// Schema version emitted and accepted by this builder release.
pub const DESCRIPTOR_DRIVER_SCHEMA_VERSION: u32 = 1;

/// One logical component bound to one selected implementation descriptor.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DescriptorDriverBinding {
    /// Stable logical component instance receiving the implementation.
    component: ComponentInstanceId,
    /// Stable implementation identity derived from its Cargo package name.
    implementation: ImplementationId,
    /// Validated host-side descriptor exported by the implementation package.
    descriptor: ComponentDescriptor,
}

impl DescriptorDriverBinding {
    /// Constructs a binding while validating its stable component and implementation identities.
    pub fn new(
        component: impl AsRef<str>,
        implementation: impl Into<Box<str>>,
        descriptor: ComponentDescriptor,
    ) -> Result<Self, InvalidStableId> {
        Ok(Self {
            component: ComponentInstanceId::new(component)?,
            implementation: ImplementationId::new(implementation)?,
            descriptor,
        })
    }
    /// Returns the bound logical component instance.
    pub fn component(&self) -> &ComponentInstanceId {
        &self.component
    }
    /// Returns the selected implementation identity.
    pub fn implementation(&self) -> &ImplementationId {
        &self.implementation
    }
    /// Returns the selected implementation descriptor.
    pub fn descriptor(&self) -> &ComponentDescriptor {
        &self.descriptor
    }
}

/// Validated result produced by one generated descriptor-driver process.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DescriptorDriverOutput {
    /// Canonical logical topology returned by the application entry point.
    topology: ApplicationTopology,
    /// Selected implementation descriptors in canonical component order.
    bindings: Box<[DescriptorDriverBinding]>,
}

impl DescriptorDriverOutput {
    /// Validates and canonically orders the generated driver's topology and bindings.
    pub fn try_new(
        topology: ApplicationTopology,
        mut bindings: Vec<DescriptorDriverBinding>,
    ) -> Result<Self, HostInterchangeError> {
        bindings.sort_by(|left, right| left.component.cmp(&right.component));
        let mut seen = BTreeSet::new();
        for binding in &bindings {
            if !seen.insert(binding.component.clone()) {
                return Err(interchange_error(format!(
                    "duplicate descriptor binding for component '{}'",
                    binding.component
                )));
            }
            let component = topology.component(&binding.component).ok_or_else(|| {
                interchange_error(format!(
                    "descriptor binding references unknown component '{}'",
                    binding.component
                ))
            })?;
            if component.contract() != binding.descriptor.contract_id()
                || component.contract_version() != binding.descriptor.contract_version()
            {
                return Err(interchange_error(format!(
                    "component '{}' requires {}@{}, but its descriptor provides {}@{}",
                    binding.component,
                    component.contract(),
                    component.contract_version(),
                    binding.descriptor.contract_id(),
                    binding.descriptor.contract_version()
                )));
            }
        }
        if let Some((component, _)) = topology
            .components()
            .find(|(component, _)| !seen.contains(*component))
        {
            return Err(interchange_error(format!(
                "component '{component}' has no descriptor binding"
            )));
        }
        Ok(Self {
            topology,
            bindings: bindings.into_boxed_slice(),
        })
    }
    /// Returns the canonical application topology.
    pub fn topology(&self) -> &ApplicationTopology {
        &self.topology
    }
    /// Returns bindings in canonical component order.
    pub fn bindings(&self) -> &[DescriptorDriverBinding] {
        &self.bindings
    }
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WireOutput {
    schema: u32,
    topology: ApplicationTopology,
    bindings: Vec<DescriptorDriverBinding>,
}
/// Encodes validated descriptor-driver output as one schema-versioned JSON document.
pub fn encode_descriptor_driver_output(
    writer: impl io::Write,
    output: DescriptorDriverOutput,
) -> Result<(), HostInterchangeError> {
    serde_json::to_writer(
        writer,
        &WireOutput {
            schema: DESCRIPTOR_DRIVER_SCHEMA_VERSION,
            topology: output.topology,
            bindings: output.bindings.into_vec(),
        },
    )
    .map_err(interchange_error)
}
/// Decodes JSON and revalidates all topology, descriptor, and binding invariants.
pub fn decode_descriptor_driver_output(
    reader: impl io::Read,
) -> Result<DescriptorDriverOutput, HostInterchangeError> {
    let wire: WireOutput = serde_json::from_reader(reader).map_err(interchange_error)?;
    if wire.schema != DESCRIPTOR_DRIVER_SCHEMA_VERSION {
        return Err(interchange_error(format!(
            "unsupported descriptor-driver schema {}; expected {DESCRIPTOR_DRIVER_SCHEMA_VERSION}",
            wire.schema
        )));
    }
    DescriptorDriverOutput::try_new(wire.topology, wire.bindings)
}
/// Invalid generated descriptor-driver output.
#[derive(Debug, Error)]
#[error("{message}")]
pub struct HostInterchangeError {
    /// Complete validation or decoding diagnostic.
    message: String,
}
/// Converts a validation or JSON failure into the interchange error wrapper.
fn interchange_error(error: impl std::fmt::Display) -> HostInterchangeError {
    HostInterchangeError {
        message: error.to_string(),
    }
}
