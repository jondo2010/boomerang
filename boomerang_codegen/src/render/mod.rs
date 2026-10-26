//! Shared source generation from validated builder models and explicit payload paths.
//! Application entry points own startup policy; these functions emit only definitions.
mod bindings;
mod image;
use crate::PayloadExports;
use anyhow::{anyhow, Context, Result};
use boomerang_builder::compiler::{
    ComponentInstanceId, FederateSlice, ImplementationId, RequiredBinding,
};
use proc_macro2::TokenStream;
use quote::quote;
use std::collections::{BTreeMap, BTreeSet};

/// Runtime capabilities installed by the application's startup code.
#[derive(Debug, Default)]
pub struct BindingConfiguration {
    /// Component instance to runtime source key, assigned by application configuration.
    pub sources: BTreeMap<ComponentInstanceId, u32>,
    /// The application installs a manual clock before execution.
    pub manual_clock: bool,
    /// Generate a binding function accepting the central RTI client bindings and sink.
    pub distributed: bool,
}
/// Format and validate generated Rust before returning it to a publisher.
pub fn format_rust(tokens: TokenStream) -> Result<String> {
    let file = syn::parse2(tokens).context("generated Rust syntax is invalid")?;
    Ok(prettyplease::unparse(&file))
}
/// Render only the canonical immutable scheduler tables for identity hashing.
pub fn image_fingerprint_input(slice: &FederateSlice<'_>) -> Result<String> {
    let mut tables = image::render_federate(slice);
    for (index, enclave) in slice.enclaves().iter().enumerate() {
        tables.extend(enclave.with_image(|image| image::render_enclave_image(index, &image)));
    }
    format_rust(tables)
}
/// Emit image tables, compatibility checks, and callable typed binding construction.
/// Each map key is the explicit implementation identity in the compiled image.
pub fn render_definitions(
    slice: &FederateSlice<'_>,
    expected_fingerprints: &BTreeMap<
        ImplementationId,
        boomerang_runtime::binding::DescriptorFingerprint,
    >,
    exports: &BTreeMap<ImplementationId, PayloadExports>,
    configuration: &BindingConfiguration,
) -> Result<TokenStream> {
    let selected = slice
        .enclaves()
        .iter()
        .flat_map(|enclave| enclave.required_bindings().iter())
        .map(RequiredBinding::implementation)
        .collect::<BTreeSet<_>>();
    let mut checks = TokenStream::new();
    for implementation in selected {
        let (Some(fingerprint), Some(payload)) = (
            expected_fingerprints.get(implementation),
            exports.get(implementation),
        ) else {
            return Err(anyhow!(
                "missing registration for implementation '{implementation}'"
            ));
        };
        let bytes = fingerprint.to_bytes();
        let bytes = bytes.iter();
        let path = &payload.path;
        checks.extend(quote! {
            const _: ()=boomerang_runtime::binding::assert_descriptor_fingerprint(boomerang_runtime::binding::DescriptorFingerprint::new([#(#bytes),*]),<#path::Payload as boomerang_runtime::binding::ComponentPayload>::DESCRIPTOR_FINGERPRINT);
        });
    }
    let mut routes = BTreeMap::new();
    let mut tables = TokenStream::new();
    for (index, enclave) in slice.enclaves().iter().enumerate() {
        enclave.with_image(|img| -> Result<()> {
            tables.extend(image::render_enclave_image(index, &img));
            bindings::collect_route_bindings(&mut routes, enclave, &img, exports)
        })?;
    }
    let federate = image::render_federate(slice);
    let bindings = bindings::render_bindings(
        slice.enclaves(),
        slice.enclave_range().start(),
        exports,
        routes,
        configuration,
    )?;
    Ok(quote! {
        use boomerang_runtime::{EnclaveBindings,FederateBindings};
        use boomerang_runtime::image::*;
        use tinymap::{IndexSpan,SliceRange,TinyMapRef};
        #checks #tables #federate #bindings
    })
}
