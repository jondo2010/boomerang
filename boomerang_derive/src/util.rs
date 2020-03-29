//! Utility newtypes and trait impls for parsing

use darling::FromMeta;
use std::convert::{TryFrom, TryInto};

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

#[derive(Debug, Clone, PartialEq)]
pub struct ExprField(syn::ExprField);
impl From<ExprField> for syn::ExprField {
    fn from(field: ExprField) -> Self {
        field.0
    }
}
impl From<syn::ExprField> for ExprField {
    fn from(field: syn::ExprField) -> Self {
        ExprField(field)
    }
}
impl FromMeta for ExprField {
    fn from_string(string: &str) -> darling::Result<Self> {
        string.try_into()
    }
    fn from_nested_meta(nm: &syn::NestedMeta) -> darling::Result<Self> {
        if let syn::NestedMeta::Lit(syn::Lit::Str(ref string)) = nm {
            string.try_into()
        } else {
            Err(darling::Error::unexpected_type("non-word").with_span(nm))
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct ExprFieldList(Vec<ExprField>);
impl From<ExprFieldList> for Vec<ExprField> {
    fn from(list: ExprFieldList) -> Self {
        list.0
    }
}
impl From<ExprFieldList> for Vec<syn::ExprField> {
    fn from(list: ExprFieldList) -> Self {
        list.0.into_iter().map(syn::ExprField::from).collect()
    }
}

/// Build an ExprPath with an ident of "self"
fn self_path() -> syn::Expr {
    syn::Expr::Path(syn::ExprPath {
        attrs: Vec::new(),
        qself: None,
        path: syn::Path {
            leading_colon: None,
            segments: vec![syn::PathSegment {
                ident: syn::Ident::new("self", proc_macro2::Span::call_site()),
                arguments: syn::PathArguments::None,
            }]
            .into_iter()
            .collect(),
        },
    })
}

impl FromMeta for ExprFieldList {
    fn from_list(items: &[syn::NestedMeta]) -> darling::Result<Self> {
        let exprs = items
            .iter()
            .map(|nmi: &syn::NestedMeta| match nmi {
                syn::NestedMeta::Lit(syn::Lit::Str(ref string)) => ExprField::try_from(string),
                _ => {
                    panic!("oops2");
                }
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(ExprFieldList(exprs))
    }
}

impl TryFrom<syn::Expr> for ExprField {
    type Error = darling::Error;
    fn try_from(expr: syn::Expr) -> Result<Self, Self::Error> {
        match expr {
            syn::Expr::Field(expr) => Ok(expr),
            syn::Expr::Path(expr) => expr
                .path
                .get_ident()
                .map(|ident| syn::ExprField {
                    attrs: vec![],
                    base: Box::new(self_path()),
                    dot_token: syn::token::Dot::default(),
                    member: syn::Member::Named(ident.to_owned()),
                })
                .ok_or(darling::Error::unexpected_type("bad")),
            _ => {
                panic!("oops1");
            }
        }
        .map(ExprField::from)
    }
}

impl TryFrom<&syn::LitStr> for ExprField {
    type Error = darling::Error;
    fn try_from(string: &syn::LitStr) -> Result<Self, Self::Error> {
        string
            .parse::<syn::Expr>()
            .map_err(|err| {
                darling::Error::unsupported_format("Error parsing expression.")
                    .with_span(&err.span())
            })
            .and_then(ExprField::try_from)
    }
}

impl TryFrom<&str> for ExprField {
    type Error = darling::Error;
    fn try_from(string: &str) -> Result<Self, Self::Error> {
        syn::parse_str::<syn::Expr>(string)
            .map_err(|err| {
                darling::Error::unsupported_format("Error parsing expression.")
                    .with_span(&err.span())
            })
            .and_then(ExprField::try_from)
    }
}
