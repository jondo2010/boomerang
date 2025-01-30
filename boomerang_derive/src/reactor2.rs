use attribute_derive::FromAttr;
use convert_case::{Case, Casing};
use proc_macro2::{Span, TokenStream};
use proc_macro_error2::abort;
use quote::{format_ident, quote, quote_spanned, ToTokens, TokenStreamExt};
use syn::{
    parse::{Parse, ParseStream},
    parse_quote,
    spanned::Spanned,
    Attribute, Block, FnArg, Ident, ItemFn, LitStr, Meta, Pat, PatIdent, ReturnType, Signature,
    Type, Visibility,
};

use crate::util::convert_from_snake_case;

/// A model that is more lenient in case of a syntax error in the function body,
/// but does not actually implement the behavior of the real model. This is
/// used to improve IDEs and rust-analyzer's auto-completion behavior in case
/// of a syntax error.
#[derive(Debug)]
pub struct DummyModel {
    pub attrs: Vec<Attribute>,
    pub vis: Visibility,
    pub sig: Signature,
    pub body: proc_macro2::TokenStream,
}

impl Parse for DummyModel {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut attrs = input.call(Attribute::parse_outer)?;
        // Drop unknown attributes like #[deprecated]
        attrs.retain(|attr| attr.path().is_ident("doc"));

        let vis: Visibility = input.parse()?;
        let sig: Signature = input.parse()?;

        // The body is left untouched, so it will not cause an error
        // even if the syntax is invalid.
        let body: proc_macro2::TokenStream = input.parse()?;

        Ok(Self {
            attrs,
            vis,
            sig,
            body,
        })
    }
}

impl ToTokens for DummyModel {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let Self {
            attrs,
            vis,
            sig,
            body,
        } = self;

        // Strip attributes like documentation comments and #[prop]
        // from the signature, so as to not confuse the user with incorrect
        // error messages.
        let sig = {
            let mut sig = sig.clone();
            sig.inputs.iter_mut().for_each(|arg| {
                if let FnArg::Typed(ty) = arg {
                    ty.attrs.retain(|attr| match &attr.meta {
                        Meta::List(list) => list
                            .path
                            .segments
                            .first()
                            .map(|n| n.ident != "prop")
                            .unwrap_or(true),
                        Meta::NameValue(name_value) => name_value
                            .path
                            .segments
                            .first()
                            .map(|n| n.ident != "doc")
                            .unwrap_or(true),
                        _ => true,
                    });
                }
            });
            sig
        };

        let output = quote! {
            #(#attrs)*
            #vis #sig #body
        };

        tokens.append_all(output)
    }
}

#[derive(Clone, Debug)]
pub struct Docs(Vec<(String, Span)>);

impl Docs {
    pub fn new(attrs: &[Attribute]) -> Self {
        #[derive(Debug, Copy, Clone, PartialEq, Eq)]
        enum ViewCodeFenceState {
            Outside,
            Rust,
            Rsx,
        }
        let mut quotes = "```".to_string();
        let mut quote_ws = "".to_string();
        let mut view_code_fence_state = ViewCodeFenceState::Outside;
        // todo fix docs stuff
        const RSX_START: &str = "# ::leptos::view! {";
        const RSX_END: &str = "# };";

        // Separated out of chain to allow rustfmt to work
        let map = |(doc, span): (String, Span)| {
            doc.split('\n')
                .map(str::trim_end)
                .flat_map(|doc| {
                    let trimmed_doc = doc.trim_start();
                    let leading_ws = &doc[..doc.len() - trimmed_doc.len()];
                    let trimmed_doc = trimmed_doc.trim_end();
                    match view_code_fence_state {
                        ViewCodeFenceState::Outside
                            if trimmed_doc.starts_with("```")
                                && trimmed_doc.trim_start_matches('`').starts_with("view") =>
                        {
                            view_code_fence_state = ViewCodeFenceState::Rust;
                            let view = trimmed_doc.find('v').unwrap();
                            trimmed_doc[..view].clone_into(&mut quotes);
                            leading_ws.clone_into(&mut quote_ws);
                            let rust_options = &trimmed_doc[view + "view".len()..].trim_start();
                            vec![
                                format!("{leading_ws}{quotes}{rust_options}"),
                                format!("{leading_ws}"),
                            ]
                        }
                        ViewCodeFenceState::Rust if trimmed_doc == quotes => {
                            view_code_fence_state = ViewCodeFenceState::Outside;
                            vec![format!("{leading_ws}"), doc.to_owned()]
                        }
                        ViewCodeFenceState::Rust if trimmed_doc.starts_with('<') => {
                            view_code_fence_state = ViewCodeFenceState::Rsx;
                            vec![format!("{leading_ws}{RSX_START}"), doc.to_owned()]
                        }
                        ViewCodeFenceState::Rsx if trimmed_doc == quotes => {
                            view_code_fence_state = ViewCodeFenceState::Outside;
                            vec![format!("{leading_ws}{RSX_END}"), doc.to_owned()]
                        }
                        _ => vec![doc.to_string()],
                    }
                })
                .map(|l| (l, span))
                .collect::<Vec<_>>()
        };

        let mut attrs = attrs
            .iter()
            .filter_map(|attr| {
                let Meta::NameValue(attr) = &attr.meta else {
                    return None;
                };
                if !attr.path.is_ident("doc") {
                    return None;
                }

                /*
                let Some(val) = value_to_string(&attr.value) else {
                    abort!(attr, "expected string literal in value of doc comment");
                };
                */

                //Some((val, attr.path.span()))
                todo!();
            })
            .flat_map(map)
            .collect::<Vec<_>>();

        if view_code_fence_state != ViewCodeFenceState::Outside {
            if view_code_fence_state == ViewCodeFenceState::Rust {
                attrs.push((quote_ws.clone(), Span::call_site()))
            } else {
                attrs.push((format!("{quote_ws}{RSX_END}"), Span::call_site()))
            }
            attrs.push((format!("{quote_ws}{quotes}"), Span::call_site()))
        }

        Self(attrs)
    }

    pub fn padded(&self) -> TokenStream {
        self.0
            .iter()
            .enumerate()
            .map(|(idx, (doc, span))| {
                let doc = if idx == 0 {
                    format!("    - {doc}")
                } else {
                    format!("      {doc}")
                };

                let doc = LitStr::new(&doc, *span);

                quote! { #[doc = #doc] }
            })
            .collect()
    }

    pub fn typed_builder(&self) -> String {
        todo!();
        /*
        let doc_str = self.0.iter().map(|s| s.0.as_str()).join("\n");

        if doc_str.chars().filter(|c| *c != '\n').count() != 0 {
            format!("\n\n{doc_str}")
        } else {
            String::new()
        }
        */
    }
}

impl ToTokens for Docs {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let s = self
            .0
            .iter()
            .map(|(doc, span)| quote_spanned!(*span=> #[doc = #doc]))
            .collect::<TokenStream>();

        tokens.append_all(s);
    }
}

#[derive(Debug)]
struct Arg {
    docs: Docs,
    kind: ArgKind,
    name: PatIdent,
    ty: Type,
}

#[derive(Debug)]
enum ArgKind {
    Input,
    Output,
    State,
    Param { default: Option<syn::Expr> },
}

impl ArgKind {
    fn from_attributes(attrs: &[Attribute]) -> syn::Result<Self> {
        let mut kind = None;
        let mut default = None;

        for attr in attrs {
            if attr.path().is_ident("input") {
                if kind.is_some() {
                    abort!(attr, "duplicate argument kind");
                }
                kind = Some(ArgKind::Input);
            } else if attr.path().is_ident("output") {
                if kind.is_some() {
                    abort!(attr, "duplicate argument kind");
                }
                kind = Some(ArgKind::Output);
            } else if attr.path().is_ident("state") {
                if kind.is_some() {
                    abort!(attr, "duplicate argument kind");
                }
                kind = Some(ArgKind::State);
            } else if attr.path().is_ident("param") {
                if default.is_some() {
                    abort!(attr, "duplicate default value");
                }

                // Check if the param attribute has arguments (for default value)
                if let Meta::List(meta_list) = &attr.meta {
                    if !meta_list.tokens.is_empty() {
                        // Parse param(default = value)
                        let nested_meta: Meta = syn::parse2(meta_list.tokens.clone())?;
                        if let Meta::NameValue(nv) = nested_meta {
                            if nv.path.is_ident("default") {
                                default = Some(nv.value);
                            } else {
                                abort!(attr, "expected 'default' in param attribute");
                            }
                        } else {
                            abort!(attr, "expected #[param(default = ...)]");
                        }
                    }
                }
            } else {
                abort!(
                    attr,
                    "unknown attribute, expected one of: input, output, state, param"
                );
            }
        }

        Ok(match kind {
            Some(k) => k,
            None => ArgKind::Param { default },
        })
    }
}

impl From<FnArg> for Arg {
    fn from(arg: FnArg) -> Self {
        let typed = if let FnArg::Typed(ty) = arg {
            ty
        } else {
            abort!(arg, "receiver not allowed in `fn`");
        };

        let kind = ArgKind::from_attributes(&typed.attrs).unwrap_or_else(|e| {
            // TODO: replace with `.unwrap_or_abort()` once https://gitlab.com/CreepySkeleton/proc-macro-error/-/issues/17 is fixed
            abort!(e.span(), e.to_string());
        });

        let name = match *typed.pat {
            Pat::Ident(i) => i,
            Pat::Struct(_) | Pat::Tuple(_) | Pat::TupleStruct(_) => {
                abort!(
                    typed.pat,
                    "destructured props must be given a name e.g. \
                         #[prop(name = \"data\")]"
                );
            }
            _ => {
                abort!(
                    typed.pat,
                    "only `prop: bool` style types are allowed within the \
                     `#[component]` macro"
                );
            }
        };

        Self {
            docs: Docs::new(&typed.attrs),
            kind,
            name,
            ty: *typed.ty,
        }
    }
}

#[derive(Debug)]
pub struct UnknownAttrs(Vec<(TokenStream, Span)>);

impl UnknownAttrs {
    pub fn new(attrs: &[Attribute]) -> Self {
        let attrs = attrs
            .iter()
            .filter_map(|attr| {
                if attr.path().is_ident("doc") {
                    if let Meta::NameValue(_) = &attr.meta {
                        return None;
                    }
                }

                Some((attr.into_token_stream(), attr.span()))
            })
            .collect();
        Self(attrs)
    }
}

#[derive(Debug)]
pub struct Model {
    docs: Docs,
    unknown_attrs: UnknownAttrs,
    vis: Visibility,
    name: Ident,
    generics: syn::Generics, // Added generics field
    args: Vec<Arg>,
    body: Box<Block>,
    ret: ReturnType,
}

impl Parse for Model {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut item = ItemFn::parse(input)?;
        //convert_impl_trait_to_generic(&mut item.sig);

        let docs = Docs::new(&item.attrs);
        let unknown_attrs = UnknownAttrs::new(&item.attrs);

        let props = item
            .sig
            .inputs
            .clone()
            .into_iter()
            .map(Arg::from)
            .collect::<Vec<_>>();

        // We need to remove the `#[doc = ""]` and `#[builder(_)]`
        // attrs from the function signature
        item.attrs.retain(|attr| match &attr.meta {
            Meta::NameValue(attr) => attr.path != parse_quote!(doc),
            Meta::List(attr) => attr.path != parse_quote!(prop),
            _ => true,
        });

        item.sig.inputs.iter_mut().for_each(|arg| {
            if let FnArg::Typed(ty) = arg {
                ty.attrs.retain(|attr| match &attr.meta {
                    Meta::NameValue(attr) => attr.path != parse_quote!(doc),
                    Meta::List(attr) => attr.path != parse_quote!(prop),
                    _ => true,
                });
            }
        });

        Ok(Self {
            docs,
            unknown_attrs,
            vis: item.vis.clone(),
            name: convert_from_snake_case(&item.sig.ident),
            generics: item.sig.generics.clone(), // Extract generics
            args: props,
            ret: item.sig.output.clone(),
            body: item.block,
        })
    }
}

impl ToTokens for Model {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let Self {
            docs,
            unknown_attrs,
            vis,
            name,
            generics,
            args,
            body,
            ret,
        } = self;

        // Extract generics parts
        let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

        // name of the generated Ports struct
        let ports_name = format_ident!("{name}Ports");
        // name of the generted State strut
        let state_name = format_ident!("{name}State");

        let port_args = args.iter().filter_map(
            |Arg {
                 docs,
                 kind,
                 name,
                 ty,
             }| match *kind {
                ArgKind::Input => Some(quote! { #[input] #docs #name: #ty }),
                ArgKind::Output => Some(quote! { #[output] #docs #name: #ty }),
                _ => None,
            },
        );

        let port_struct = quote! {
            #[reactor_ports]
            #vis struct #ports_name #impl_generics #where_clause {
                #(#port_args),*
            }
        };

        let state_args = args
            .iter()
            .filter_map(
                |Arg {
                     docs,
                     kind,
                     name,
                     ty,
                 }| match *kind {
                    ArgKind::State => Some(quote! { #docs #name: #ty }),
                    _ => None,
                },
            )
            .collect::<Vec<_>>();

        let state_struct = if state_args.is_empty() {
            quote! {
                #vis type #state_name = ();
            }
        } else {
            quote! {
                #vis struct #state_name #impl_generics #where_clause {
                    #(#state_args),*
                }
            }
        };

        let port_idents = args
            .iter()
            .filter_map(|Arg { kind, name, .. }| match *kind {
                ArgKind::Input | ArgKind::Output => Some(name),
                _ => None,
            });

        let ret = quote! { -> impl ::boomerang::builder::Reactor2<#state_name #ty_generics, Ports = #ports_name #ty_generics> };

        let output = quote! {
            #port_struct
            #state_struct

            #docs
            #vis fn #name #impl_generics() #ret #where_clause {
                <#ports_name #ty_generics as ::boomerang::builder::ReactorPorts>::build_with::<_, #state_name #ty_generics>(
                    |builder, (#(#port_idents,)*)| {
                        #body
                        Ok(())
                    })
            }
        };

        tokens.append_all(output)
    }
}
