//! Generate the declared state initializer context for each compilation facet.

use super::{ArgKind, Model, ReactorArgs};
use proc_macro2::TokenStream;
use quote::quote;

/// Arguments for the `init_context` reactor option.
#[derive(attribute_derive::FromAttr)]
pub(super) struct InitContextArgs {
    /// User-chosen Rust name of the generated bundle.
    pub(super) name: syn::Ident,
    /// Component-local source capability label; deployment binds each instance.
    pub(super) source: Option<syn::LitStr>,
    /// Existing physical action identifiers in user-declared field order.
    pub(super) actions: Option<syn::ExprArray>,
    /// Explicit manual clock write authority.
    pub(super) clock_control: Option<syn::LitBool>,
}
impl InitContextArgs {
    /// Resolves declared names against existing physical actions before code generation.
    pub(super) fn actions<'a>(
        &self,
        model: &'a Model,
    ) -> syn::Result<Vec<(&'a syn::Ident, &'a syn::Type)>> {
        let mut result = Vec::new();
        if let Some(actions) = &self.actions {
            for action in &actions.elems {
                let syn::Expr::Path(path) = action else {
                    return Err(syn::Error::new_spanned(
                        action,
                        "init_context actions must be physical action identifiers",
                    ));
                };
                let Some(ident) = path.path.get_ident() else {
                    return Err(syn::Error::new_spanned(
                        action,
                        "init_context action must be a local physical action",
                    ));
                };
                let arg = model
                    .args
                    .iter()
                    .find(|arg| {
                        arg.name.ident == *ident
                            && matches!(arg.kind, ArgKind::PhysicalAction { .. })
                    })
                    .ok_or_else(|| {
                        syn::Error::new_spanned(
                            action,
                            "init_context action is missing or is not physical",
                        )
                    })?;
                if ["source", "clock", "send", "__boomerang_marker"]
                    .iter()
                    .any(|reserved| ident == reserved)
                    || result.iter().any(|(name, _)| *name == ident)
                {
                    return Err(syn::Error::new_spanned(
                        action,
                        "duplicate or reserved init_context field",
                    ));
                }
                result.push((&arg.name.ident, &arg.ty));
            }
        }
        Ok(result)
    }
    /// Whether explicit manual clock control is requested.
    pub(super) fn clock_control(&self) -> bool {
        self.clock_control.as_ref().is_some_and(|value| value.value)
    }
}
/// Keeps reactor parameters that do not occur in the selected action types.
fn generic_marker(model: &Model) -> (TokenStream, TokenStream) {
    if model.generics.params.is_empty() {
        return (TokenStream::new(), TokenStream::new());
    }
    let types = model
        .generics
        .params
        .iter()
        .filter_map(|param| match param {
            syn::GenericParam::Type(param) => {
                let name = &param.ident;
                Some(quote!(*const #name))
            }
            syn::GenericParam::Lifetime(param) => {
                let lifetime = &param.lifetime;
                Some(quote!(&#lifetime ()))
            }
            syn::GenericParam::Const(_) => None,
        });
    (
        quote!(__boomerang_marker: ::core::marker::PhantomData<fn() -> (#(#types,)*)>,),
        quote!(__boomerang_marker: ::core::marker::PhantomData,),
    )
}

/// Emits the public user-named argument structure in dynamic and payload facets.
pub(super) fn declaration(args: &ReactorArgs, model: &Model) -> syn::Result<TokenStream> {
    let Some(capabilities) = &args.init_context else {
        return Ok(TokenStream::new());
    };
    let name = &capabilities.name;
    let generics = &model.generics;
    let where_clause = &generics.where_clause;
    let (marker, _) = generic_marker(model);
    let actions = capabilities.actions(model)?.into_iter().map(|(field, ty)| {
        let (ty, description) = if capabilities.source.is_some() {
            (
                quote!(::boomerang::runtime::input::SourceAction<#ty>),
                "Source-owned action for timestamp-preserving admission through InputSourceHandle.",
            )
        } else {
            (
                quote!(::boomerang::runtime::AsyncActionRef<#ty>),
                "Ordinary asynchronous action for SendContext scheduling.",
            )
        };
        quote!(#[doc = #description] pub #field: #ty,)
    });
    let source = capabilities.source.as_ref().map(|_| quote!(#[doc = "Execution-bound source progress and timestamped input admission."] pub source: ::boomerang::runtime::input::InputSourceHandle,));
    let clock = capabilities.clock_control().then(|| quote!(#[doc = "Explicit write authority for the selected manual clock."] pub clock: ::boomerang::runtime::clock::ManualClock,));
    Ok(quote! {
        /// Context supplied after validation and before logical startup.
        /// Return without waiting for logical execution. Keep worker guards in state.
        pub struct #name #generics #where_clause {
            #marker
            #source #clock #(#actions)*
            /// Ordinary asynchronous sending. Use nonblocking sends during initialization.
            pub send: ::boomerang::runtime::SendContext,
        }
    })
}
/// Emits a generated adapter from compiler-resolved rows to the user's named fields.
pub(super) fn initializer(
    args: &ReactorArgs,
    model: &Model,
    symbol: &syn::Ident,
    state: &syn::Path,
) -> syn::Result<TokenStream> {
    let capabilities = args.init_context.as_ref().expect("capability declaration");
    let name = &capabilities.name;
    let path = args.state_init.as_ref().expect("validated initializer");
    let (impl_generics, ty_generics, where_clause) = model.generics.split_for_impl();
    let type_args = ty_generics.as_turbofish();
    let (_, marker) = generic_marker(model);
    let fields =
        capabilities
            .actions(model)?
            .into_iter()
            .enumerate()
            .map(|(index, (field, ty))| {
                let method = if capabilities.source.is_some() {
                    quote!(source_action)
                } else {
                    quote!(async_action)
                };
                quote!(#field: context.#method::<#ty>(actions[#index])?,)
            });
    let source = capabilities
        .source
        .as_ref()
        .map(|_| quote!(source: context.source()?,));
    let clock = capabilities
        .clock_control()
        .then(|| quote!(clock: context.clock_control()?,));
    Ok(quote! {
        /// Constructs typed capabilities from validated existing action rows.
        #[allow(non_snake_case)]
        pub fn #symbol #impl_generics(context: &::boomerang::runtime::initialization::StateInitContext<'_, '_>, actions: &[::boomerang::runtime::image::ActionIndex]) -> Result<#state, ::boomerang::runtime::initialization::StateInitializationError> #where_clause {
            Ok(#path(#name #type_args { #marker #source #clock #(#fields)* send: context.send_context() })?)
        }
    })
}
/// Registers the same named initializer in the live builder using existing action aliases.
pub(super) fn live_registration(args: &ReactorArgs, model: &Model) -> syn::Result<TokenStream> {
    let Some(capabilities) = &args.init_context else {
        return Ok(quote!());
    };
    if capabilities.source.is_some() || capabilities.clock_control() {
        return Ok(quote! {
            if ctx.needs_initialization() { return Err(::boomerang::builder::AssemblyError::InconsistentAssemblyState { what: "source admission and manual clock capabilities require compiled Federate execution".into() }); }
        });
    }
    let name = &capabilities.name;
    let init = args.state_init.as_ref().expect("validated initializer");
    let (_, ty_generics, _) = model.generics.split_for_impl();
    let type_args = ty_generics.as_turbofish();
    let (_, marker) = generic_marker(model);
    let actions = capabilities.actions(model)?;
    let aliases = actions.iter().map(|(field, _)| quote!(let (_, #field) = runtime.aliases.action_aliases[::boomerang::builder::AssemblyActionKey::from(#field)];));
    let requirements = actions.iter().map(|(field, ty)| quote!(::boomerang::runtime::initialization::live::LiveActionCapability::new::<#ty>(#field)));
    let fields = actions
        .iter()
        .map(|(field, ty)| quote!(#field: context.async_action::<#ty>(#field)?,));
    Ok(quote! {
        ctx.initialize_with(move |name, runtime| {
            #(#aliases)*
            ::boomerang::runtime::initialization::live::deferred_reactor(name, vec![#(#requirements),*], move |context| {
                #init(#name #type_args { #marker #(#fields)* send: context.send_context() }).map_err(::core::convert::Into::into)
            })
        });
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Declarative action references must reject missing, logical, duplicate and reserved fields.
    #[test]
    fn capability_action_declarations_reject_invalid_fields() {
        let model: Model = syn::parse_quote! {
            fn Sensor(#[physical_action] sample: u32, #[logical_action] logical: u32, #[physical_action] send: u32, #[physical_action] __boomerang_marker: u32) -> impl Reactor {}
        };
        for (actions, expected) in [
            (syn::parse_quote!([missing]), "missing or is not physical"),
            (syn::parse_quote!([logical]), "missing or is not physical"),
            (syn::parse_quote!([sample, sample]), "duplicate or reserved"),
            (syn::parse_quote!([send]), "duplicate or reserved"),
            (
                syn::parse_quote!([__boomerang_marker]),
                "duplicate or reserved",
            ),
        ] {
            let declaration = InitContextArgs {
                name: syn::parse_quote!(DriverInit),
                source: Some(syn::parse_quote!("plant")),
                actions: Some(actions),
                clock_control: None,
            };
            assert!(declaration
                .actions(&model)
                .unwrap_err()
                .to_string()
                .contains(expected));
        }
    }
}
