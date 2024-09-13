use std::time::Duration;

use quote::{quote, ToTokens};
use syn::{Ident, Type};

pub fn extract_path_ident(elem: &Type) -> Option<&Ident> {
    match elem {
        Type::Path(syn::TypePath {
            path: syn::Path { segments, .. },
            ..
        }) => segments.last().map(|segment| &segment.ident),
        Type::Reference(syn::TypeReference { elem, .. }) => extract_path_ident(elem),
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
