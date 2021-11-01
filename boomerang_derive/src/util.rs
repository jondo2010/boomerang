//! Utility newtypes and trait impls for parsing

use darling::FromMeta;
use derive_more::Display;
use proc_macro2::Ident;
use quote::quote;
use std::{collections::HashSet, time::Duration};
use syn::ext::IdentExt;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct StringList(Vec<String>);
impl std::ops::Deref for StringList {
    type Target = Vec<String>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<StringList> for Vec<String> {
    fn from(string_list: StringList) -> Self {
        string_list.0
    }
}

impl FromMeta for StringList {
    fn from_list(v: &[syn::NestedMeta]) -> darling::Result<Self> {
        let mut strings = Vec::with_capacity(v.len());
        for nmi in v {
            if let syn::NestedMeta::Lit(syn::Lit::Str(ref string)) = *nmi {
                strings.push(string.value().clone());
            } else {
                return Err(darling::Error::unexpected_type("non-word").with_span(nmi));
            }
        }
        Ok(StringList(strings))
    }
}

pub struct Type(syn::Type);
impl FromMeta for Type {
    fn from_string(value: &str) -> darling::Result<Self> {
        syn::parse_str::<syn::Type>(value)
            .map_err(|err| {
                darling::Error::unsupported_format("Error parsing expression.")
                    .with_span(&err.span())
            })
            .map(Self::from)
    }
}
impl From<Type> for syn::Type {
    fn from(ty: Type) -> Self {
        ty.0
    }
}
impl From<syn::Type> for Type {
    fn from(ty: syn::Type) -> Self {
        Type(ty)
    }
}

pub struct Expr(syn::Expr);
impl FromMeta for Expr {
    fn from_string(value: &str) -> darling::Result<Self> {
        syn::parse_str::<syn::Expr>(value)
            .map_err(|err| {
                darling::Error::unsupported_format("Error parsing expression.")
                    .with_span(&err.span())
            })
            .map(Self::from)
    }
}
impl From<Expr> for syn::Expr {
    fn from(expr: Expr) -> Self {
        expr.0
    }
}
impl From<syn::Expr> for Expr {
    fn from(expr: syn::Expr) -> Expr {
        Expr(expr)
    }
}

pub fn handle_ident(string: syn::LitStr) -> syn::Ident {
    syn::Ident::new(&string.value(), string.span())
}

#[derive(Debug, Clone, PartialEq)]
pub struct IdentSet(HashSet<syn::Ident>);
impl From<IdentSet> for HashSet<syn::Ident> {
    fn from(list: IdentSet) -> Self {
        list.0
    }
}
impl FromMeta for IdentSet {
    fn from_list(items: &[syn::NestedMeta]) -> darling::Result<Self> {
        let exprs = items
            .iter()
            .map(|nmi: &syn::NestedMeta| match nmi {
                syn::NestedMeta::Lit(syn::Lit::Str(string)) => {
                    Ok(handle_ident(string.clone()))
                    // string.parse::<syn::Ident>().map_err(|err| {})
                }
                _ => {
                    Err(darling::Error::unsupported_format(
                        "Error parsing expression.",
                    )) //.with_span(&err.span())
                }
            })
            .collect::<Result<HashSet<_>, _>>()?;

        Ok(IdentSet(exprs))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Display)]
#[display(fmt = "{}.{}", "0.to_string()", "1.to_string()")]
pub struct NamedField(pub Ident, pub Ident);

impl syn::parse::Parse for NamedField {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        use syn::Token;
        if input.peek2(Token![.]) {
            let base = Ident::parse_any(input)?;
            let _ = input.parse::<Token![.]>()?;
            let member = Ident::parse_any(input)?;
            Ok(NamedField(base, member))
        } else {
            let base = Ident::new("self", input.span());
            let member = Ident::parse_any(input)?;
            Ok(NamedField(base, member))
        }
    }
}

impl FromMeta for NamedField {
    fn from_string(string: &str) -> darling::Result<Self> {
        syn::parse_str(string)
            .map_err(|err| darling::Error::custom("Error parsing value").with_span(&err.span()))
    }
}

#[test]
fn test_named_field() {
    let input = syn::parse_str(r#""source.out""#).unwrap();
    let field: NamedField = FromMeta::from_nested_meta(&input).unwrap();
    assert_eq!(field, syn::parse_quote!(source.out));

    let input = syn::parse_str(r#""lonely""#).unwrap();
    let field: NamedField = FromMeta::from_nested_meta(&input).unwrap();
    assert_eq!(field, syn::parse_quote!(self.lonely));

    let field: NamedField = syn::parse_str("foo.impl").unwrap();
    assert_eq!(field, syn::parse_quote!(foo.impl));
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct MetaList<T>(Vec<T>);

impl<T> From<MetaList<T>> for Vec<T> {
    fn from(list: MetaList<T>) -> Self {
        list.0
    }
}

impl<T: FromMeta> FromMeta for MetaList<T> {
    fn from_list(items: &[syn::NestedMeta]) -> darling::Result<Self> {
        let exprs = items
            .iter()
            .map(T::from_nested_meta)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(MetaList(exprs))
    }
}

/// Generate a TokenSTream from an Option<Duration>
pub(crate) fn duration_quote(duration: &Duration) -> proc_macro2::TokenStream {
    let secs = duration.as_secs();
    let nanos = duration.subsec_nanos();
    quote! {::boomerang::runtime::Duration::new(#secs, #nanos)}
}
