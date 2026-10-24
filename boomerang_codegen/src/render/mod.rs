//! Shared source generation from validated builder models and explicit payload paths.
//! Application entry points own startup policy; these functions emit only definitions.
mod bindings;
mod image;
use crate::{ComponentRegistration, ManifestStyle};
use anyhow::{anyhow, Context, Result};
use boomerang_builder::compiler::{
    ComponentInstanceId, FederateSlice, ImplementationId, RequiredBinding,
};
use proc_macro2::TokenStream;
use quote::quote;
use std::collections::BTreeMap;

/// Runtime capabilities installed by the application's startup code.
#[derive(Debug, Clone, Default)]
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
    components: &BTreeMap<ImplementationId, ComponentRegistration>,
    configuration: &BindingConfiguration,
) -> Result<TokenStream> {
    for required in slice
        .enclaves()
        .iter()
        .flat_map(|e| e.required_bindings().iter())
    {
        let implementation = match required {
            RequiredBinding::State { implementation, .. }
            | RequiredBinding::Reaction { implementation, .. }
            | RequiredBinding::Port { implementation, .. }
            | RequiredBinding::Action { implementation, .. } => implementation,
        };
        if !components.contains_key(implementation) {
            return Err(anyhow!(
                "missing registration for implementation '{implementation}', binding '{}'",
                required.symbol()
            ));
        }
    }
    let mut routes = BTreeMap::new();
    let mut tables = TokenStream::new();
    for (index, enclave) in slice.enclaves().iter().enumerate() {
        enclave.with_image(|img| -> Result<()> {
            tables.extend(image::render_enclave_image(index, &img));
            bindings::collect_route_bindings(&mut routes, enclave, &img, components)
        })?;
    }
    let federate = image::render_federate(slice);
    let bindings = bindings::render_bindings(
        components,
        slice.enclaves(),
        slice.enclave_range().start(),
        components,
        routes,
        configuration.distributed,
        configuration,
    )?;
    let checks=components.values().map(|registration| {
        let descriptor=&registration.descriptor; let bytes=descriptor.fingerprint().to_bytes(); let bytes=bytes.iter(); let abi=descriptor.macro_abi(); let path=&registration.payload.path;
        let manifest=match registration.payload.manifest {ManifestStyle::Function=>quote!(#path::binding_manifest()),ManifestStyle::Constant=>quote!(#path::BINDING_MANIFEST)};
        quote! {
            const _: ()=boomerang_runtime::binding::assert_descriptor_fingerprint(boomerang_runtime::binding::DescriptorFingerprint::new([#(#bytes),*]),#manifest.descriptor_fingerprint());
            const _: ()=assert!(#abi==#manifest.macro_abi(),"macro ABI mismatch");
        }
    });
    Ok(quote! {
        use boomerang_runtime::{EnclaveBindings,FederateBindings};
        use boomerang_runtime::image::*;
        use tinymap::{IndexSpan,SliceRange,TinyMapRef};
        #(#checks)* #tables #federate #bindings
    })
}
