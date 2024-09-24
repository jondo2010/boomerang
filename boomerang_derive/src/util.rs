use std::time::Duration;

use quote::{quote, ToTokens};
use syn::{Ident, Type};

/// Recursively expand an expression into a list of identifiers from the Path and Field expressions
pub fn expand_expr(expr: &syn::Expr, idents: &mut Vec<syn::Ident>) -> Result<(), darling::Error> {
    match expr {
        syn::Expr::Field(field) => {
            expand_expr(&field.base, idents)?;
            if let syn::Member::Named(ident) = &field.member {
                idents.push(ident.clone())
            }
        }
        syn::Expr::Path(path) => {
            if let Some(ident) = path.path.get_ident() {
                idents.push(ident.clone());
            }
        }
        _ => {
            return Err(darling::Error::custom(
                "Unsupported expression type {expr:?}",
            ));
        }
    }
    Ok(())
}

pub fn extract_path_ident(elem: &Type) -> Option<&Ident> {
    match elem {
        Type::Path(syn::TypePath {
            path: syn::Path { segments, .. },
            ..
        }) => segments.last().map(|segment| &segment.ident),
        Type::Reference(syn::TypeReference { elem, .. }) => extract_path_ident(elem),

        Type::Array(syn::TypeArray { elem, .. }) => extract_path_ident(elem),
        _ => None,
    }
}

pub fn handle_duration(value: String) -> Option<Duration> {
    Some(parse_duration::parse(&value).unwrap())
}

/// Generate a TokenStream from an Option<Duration>
pub fn duration_quote(duration: &Duration) -> proc_macro2::TokenStream {
    let secs = duration.as_secs();
    let nanos = duration.subsec_nanos();
    quote! {::boomerang::runtime::Duration::new(#secs, #nanos)}
}

pub struct OptionalDuration(pub Option<Duration>);

impl ToTokens for OptionalDuration {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        tokens.extend(match self.0 {
            Some(duration) => {
                let duration = duration_quote(&duration);
                quote! {Some(#duration)}
            }
            None => quote! {None},
        });
    }
}
