use super::BindingConfiguration;
use crate::{ComponentRegistration, PayloadExports};
use anyhow::{anyhow, bail, Context, Result};
use boomerang_builder::compiler::{
    ComponentInstanceId, ImplementationId, OwnedEnclaveImage, RequiredBinding,
};
use boomerang_runtime::image::{EnclaveImage, RouteDirection};
use proc_macro2::TokenStream;
use quote::quote;
use std::collections::BTreeMap;

pub(super) fn render_bindings(
    enclaves: &[OwnedEnclaveImage],
    enclave_start: usize,
    exports: &BTreeMap<ImplementationId, ComponentRegistration>,
    route_bindings: RouteBindings,
    distributed: bool,
    physical: &BindingConfiguration,
) -> Result<TokenStream> {
    let inputs = &physical.sources;
    let manual = physical.manual_clock;
    let writers = enclaves.iter().flat_map(|enclave| enclave.required_bindings().iter()).filter(|binding| {
        matches!(binding, RequiredBinding::State { initializer: Some(required), .. } if required.clock_control)
    }).count();
    if writers > 1 {
        bail!("manual clock control must have one component owner");
    }
    let mut enclave_bindings = Vec::with_capacity(enclaves.len());
    for (enclave_offset, enclave) in enclaves.iter().enumerate() {
        let enclave_index = u32::try_from(
            enclave_start
                .checked_add(enclave_offset)
                .ok_or_else(|| anyhow!("compiled Enclave index exceeds the image domain"))?,
        )?;
        let enclave_index = proc_macro2::Literal::u32_unsuffixed(enclave_index);
        let mut bindings = TokenStream::new();
        for (slot, binding) in enclave.required_bindings().iter().enumerate() {
            bindings.extend(
                render_binding(exports, slot, binding, inputs, manual)
                    .map_err(|error| crate::CompilationError::render_binding(error, binding))?,
            );
        }
        enclave_bindings.push(quote!(
            .bind_enclave(
                EnclaveIndex::new(#enclave_index),
                EnclaveBindings::new() #bindings,
            )
        ));
    }
    let route_bindings = render_route_bindings(route_bindings);
    let body = quote!(FederateBindings::new() #(#enclave_bindings)* #route_bindings);
    Ok(if distributed {
        quote! {
            pub fn generated_bindings(
                _rti: &boomerang_central_rti::compiled::RtiClientBindings<'static>,
                _sink: std::sync::Arc<dyn boomerang_central_rti::compiled::RtiRequestSink>,
            ) -> Result<FederateBindings<'static>, Box<dyn std::error::Error>> {
                Ok(#body)
            }
        }
    } else {
        quote!(pub fn generated_bindings() -> FederateBindings<'static> { #body })
    })
}

/// Accumulated generated payload types for each local scheduler boundary.
pub(super) type RouteBindings = BTreeMap<String, (Option<TokenStream>, Option<TokenStream>)>;

/// Validates one externally derived Rust identifier before token interpolation.
fn rust_ident(value: &str, role: &str) -> Result<syn::Ident> {
    syn::parse_str(value).with_context(|| format!("invalid generated {role} identifier '{value}'"))
}

/// Renders a generated payload binding path from validated identifier segments.
fn binding_path(alias: &PayloadExports, symbol: &str) -> Result<TokenStream> {
    let alias = &alias.path;
    let symbol = rust_ident(symbol, "binding symbol")?;
    Ok(quote!(#alias::#symbol))
}

/// Collects paired route payloads while each enclave image is borrowed.
pub(super) fn collect_route_bindings(
    routes: &mut RouteBindings,
    enclave: &OwnedEnclaveImage,
    image: &EnclaveImage<'_>,
    exports: &BTreeMap<ImplementationId, ComponentRegistration>,
) -> Result<()> {
    for route in image.routes.values() {
        let boundary = route.boundary().as_str().to_owned();
        let port = &image.ports[route.local_port()];
        let binding = enclave
            .required_bindings()
            .get(port.binding())
            .expect("compiled port binding index is validated");
        let RequiredBinding::Port { implementation, .. } = binding else {
            bail!("route '{boundary}' local port does not have a port binding");
        };
        let alias = exports.get(implementation).ok_or_else(|| {
            anyhow!("missing generated alias for implementation '{implementation}'")
        })?;
        let payload = binding_path(&alias.payload, &binding.symbol())?;
        let pair = routes.entry(boundary.clone()).or_default();
        let slot = match route.direction() {
            RouteDirection::Outbound => &mut pair.0,
            RouteDirection::Inbound => &mut pair.1,
        };
        if slot.replace(payload).is_some() {
            bail!(
                "route '{boundary}' has duplicate {:?} halves",
                route.direction()
            );
        }
    }
    Ok(())
}

/// Emits one typed route binding for each paired local scheduler boundary.
fn render_route_bindings(routes: RouteBindings) -> TokenStream {
    let mut tokens = TokenStream::new();
    for (boundary, (source_payload, destination_payload)) in routes {
        tokens.extend(match (source_payload, destination_payload) {
            (Some(source), Some(destination)) => quote! {
                .bind_route(BoundaryId::new(#boundary), #source, #destination)
            },
            (Some(source), None) => quote! {
                .bind_outbound_route(
                    BoundaryId::new(#boundary), #source,
                    boomerang_central_rti::compiled::hosted::encode_json,
                    _rti.outbound_sink(_sink.clone(), BoundaryId::new(#boundary))?,
                )
            },
            (None, Some(destination)) => quote! {
                .bind_inbound_route(
                    BoundaryId::new(#boundary), #destination,
                    boomerang_central_rti::compiled::hosted::decode_json,
                )
            },
            (None, None) => TokenStream::new(),
        });
    }
    tokens
}

/// Emits one state, reaction, port, or action binding chain entry.
fn render_binding(
    exports: &BTreeMap<ImplementationId, ComponentRegistration>,
    slot: usize,
    binding: &RequiredBinding,
    inputs: &BTreeMap<ComponentInstanceId, u32>,
    manual: bool,
) -> Result<TokenStream> {
    let slot = u32::try_from(slot).context("generated binding slot exceeds u32 range")?;
    let slot = proc_macro2::Literal::u32_unsuffixed(slot);
    let (implementation, symbol) = match binding {
        RequiredBinding::State { implementation, .. }
        | RequiredBinding::Reaction { implementation, .. }
        | RequiredBinding::Port { implementation, .. }
        | RequiredBinding::Action { implementation, .. } => (implementation, binding.symbol()),
    };
    let alias = exports
        .get(implementation)
        .ok_or_else(|| anyhow!("missing generated alias for implementation '{implementation}'"))?;
    let path = binding_path(&alias.payload, &symbol)?;
    match binding {
        RequiredBinding::State {
            component,
            initializer,
            ..
        } => {
            let Some(required) = initializer else {
                return Ok(quote!(.bind_state(BindingSlotIndex::new(#slot), #path)));
            };
            if required.clock_control && !manual {
                bail!("component '{component}' requires a selected manual clock")
            }
            let clock_control = required.clock_control;
            let source = if let Some(source) = &required.source {
                let index = inputs.get(component).copied().ok_or_else(|| {
                    anyhow!("component '{component}' requires physical source '{source}'")
                })?;
                quote!(source: Some(boomerang_runtime::input::SourceKey::new(#index)),)
            } else {
                quote!()
            };
            let mut action_rows = Vec::new();
            let mut requirements = Vec::new();
            for action in &required.actions {
                let symbol = boomerang_builder::compiler::direct_binding_symbol(
                    boomerang_runtime::image::BindingKind::Action,
                    action.slot.path(),
                );
                let payload = binding_path(&alias.payload, &symbol)?;
                let row = action.index.as_u32();
                action_rows.push(quote!(ActionIndex::new(#row)));
                requirements.push(quote!(boomerang_runtime::initialization::ActionCapability::new(ActionIndex::new(#row), #payload)));
            }
            Ok(
                quote!(.bind_state_with_capabilities(BindingSlotIndex::new(#slot), boomerang_runtime::initialization::StateCapabilities { #source actions: vec![#(#requirements),*], clock_control: #clock_control, ..Default::default() }, |context| #path(context, &[#(#action_rows),*]))),
            )
        }
        RequiredBinding::Port { .. } => Ok(quote!(
            .bind_port(BindingSlotIndex::new(#slot), #path)
        )),
        RequiredBinding::Action { .. } => Ok(quote!(
            .bind_action(BindingSlotIndex::new(#slot), #path)
        )),
        RequiredBinding::Reaction { .. } => Ok(quote!(
            .bind_reaction(BindingSlotIndex::new(#slot), #path)
        )),
    }
}

#[cfg(test)]
mod tests {
    use quote::quote;

    use crate::render::format_rust;

    /// Confirms malformed structural output fails at the publication boundary.
    #[test]
    fn malformed_generated_tokens_are_rejected_before_publication() {
        let error = format_rust(quote!(fn incomplete)).unwrap_err();

        assert!(
            error.to_string().contains("generated Rust syntax"),
            "{error:#}"
        );
    }
}
