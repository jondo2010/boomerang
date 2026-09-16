use proc_macro2::TokenStream;
use quote::{quote, ToTokens};

/// Expands the owning inline module without relocating any helper items.
pub fn expand(input: TokenStream) -> syn::Result<TokenStream> {
    let module: syn::ItemMod = syn::parse2(input)?;
    let Some((_, items)) = module.content else {
        return Err(syn::Error::new_spanned(
            module,
            "component! requires an inline module",
        ));
    };
    let mut reactor_found = false;
    let mut output = Vec::new();
    for item in items {
        if let syn::Item::Fn(mut function) = item {
            let attributes: Vec<_> = function
                .attrs
                .iter()
                .enumerate()
                .filter(|(_, attribute)| {
                    attribute
                        .path()
                        .segments
                        .last()
                        .is_some_and(|segment| segment.ident == "reactor")
                })
                .map(|(index, _)| index)
                .collect();
            if !attributes.is_empty() {
                if reactor_found || attributes.len() != 1 {
                    return Err(syn::Error::new_spanned(
                        function,
                        "component! requires exactly one root #[reactor] declaration",
                    ));
                }
                let attribute = function.attrs.remove(attributes[0]);
                if let Some(unsupported) = function
                    .attrs
                    .iter()
                    .find(|attribute| !attribute.path().is_ident("doc"))
                {
                    return Err(syn::Error::new_spanned(unsupported,
                        "unsupported component root attribute; place conditional or lint attributes on the component module instead"));
                }
                let args = match attribute.meta {
                    syn::Meta::Path(_) => syn::parse2(TokenStream::new())?,
                    syn::Meta::List(list) => syn::parse2(list.tokens)?,
                    other => {
                        return Err(syn::Error::new_spanned(other, "expected #[reactor(...)]"))
                    }
                };
                let model = syn::parse2(function.into_token_stream())?;
                reactor_found = true;
                output.push(crate::reactor::ArgsModel(args, model, true).into_token_stream());
            } else {
                output.push(helper(function.into_token_stream()));
            }
        } else if matches!(item, syn::Item::Use(_)) {
            // Imports used by hosted reactor expansion may disappear in payload mode.
            output.push(helper(quote!(#[allow(unused_imports)] #item)));
        } else {
            output.push(helper(item.into_token_stream()));
        }
    }
    if !reactor_found {
        return Err(syn::Error::new_spanned(
            &module.ident,
            "component! requires exactly one root #[reactor] declaration",
        ));
    };
    let (outer_attrs, inner_attrs): (Vec<_>, Vec<_>) = module
        .attrs
        .into_iter()
        .partition(|attribute| matches!(attribute.style, syn::AttrStyle::Outer));
    let vis = module.vis;
    let name = module.ident;
    Ok(quote! {
        #(#outer_attrs)*
        #[allow(unexpected_cfgs)]
        #vis mod #name {
            #(#inner_attrs)*
            #(#output)*
        }
    })
}

fn helper(item: TokenStream) -> TokenStream {
    quote! {
        #[cfg(not(any(boomerang_facet = "descriptor", feature = "__boomerang_descriptor")))]
        #item
    }
}
