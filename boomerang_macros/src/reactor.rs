use proc_macro2::{Span, TokenStream};
use proc_macro_error2::abort;
use quote::{format_ident, quote, quote_spanned, ToTokens, TokenStreamExt};
use syn::{
    parse::Parse, parse_quote, spanned::Spanned, Attribute, Block, FnArg, Ident, ItemFn, Meta, Pat,
    PatIdent, Type, TypePath, Visibility,
};

use crate::util::convert_from_snake_case;

/// Top-level arguments for the #[reactor] macro
#[derive(attribute_derive::FromAttr)]
pub struct ReactorArgs {
    /// The name of the state type to use. If not provided, a state struct will be generated.
    state: Option<TypePath>,
}

#[derive(Clone, Debug)]
pub struct Docs(Vec<(String, Span)>);

impl Docs {
    pub fn new(attrs: &[Attribute]) -> Self {
        let docs = attrs
            .iter()
            .filter_map(|attr| {
                let Meta::NameValue(attr) = &attr.meta else {
                    return None;
                };
                if !attr.path.is_ident("doc") {
                    return None;
                }

                // Extract the string value from the doc attribute
                let val = match &attr.value {
                    syn::Expr::Lit(syn::ExprLit {
                        lit: syn::Lit::Str(lit_str),
                        ..
                    }) => lit_str.value(),
                    _ => {
                        abort!(attr, "expected string literal in value of doc comment");
                    }
                };

                Some((val, attr.path.span()))
            })
            .collect::<Vec<_>>();

        Self(docs)
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
    Input { len: Option<syn::Expr> },
    Output { len: Option<syn::Expr> },
    State { default: Option<syn::Expr> },
    Param { default: Option<syn::Expr> },
}

impl ArgKind {
    fn from_attributes(attrs: &[Attribute]) -> syn::Result<Self> {
        let mut kind = None;

        for attr in attrs {
            if attr.path().is_ident("input") {
                if kind.is_some() {
                    abort!(attr, "duplicate argument kind");
                }
                let port_len = parse_len(attr)?;
                kind = Some(ArgKind::Input { len: port_len });
            } else if attr.path().is_ident("output") {
                if kind.is_some() {
                    abort!(attr, "duplicate argument kind");
                }
                let port_len = parse_len(attr)?;
                kind = Some(ArgKind::Output { len: port_len });
            } else if attr.path().is_ident("state") {
                if kind.is_some() {
                    abort!(attr, "duplicate argument kind");
                }
                let default = parse_default(attr)?;
                kind = Some(ArgKind::State { default });
            } else if attr.path().is_ident("param") {
                if kind.is_some() {
                    abort!(attr, "duplicate argument kind");
                }
                let default = parse_default(attr)?;
                kind = Some(ArgKind::Param { default });
            } else {
                abort!(
                    attr,
                    "unknown attribute, expected one of: input, output, state, param"
                );
            }
        }

        Ok(match kind {
            Some(k) => k,
            None => ArgKind::Param { default: None },
        })
    }
}

fn parse_default(attr: &Attribute) -> syn::Result<Option<syn::Expr>> {
    let meta_list = match &attr.meta {
        Meta::List(list) => list,
        Meta::Path(_) => return Ok(None),
        _ => {
            return Err(syn::Error::new_spanned(
                attr,
                "expected #[param] or #[param(default = ...)]",
            ))
        }
    };

    if meta_list.tokens.is_empty() {
        return Ok(None);
    }

    let nested_meta: Meta = syn::parse2(meta_list.tokens.clone())?;
    if let Meta::NameValue(nv) = nested_meta {
        if nv.path.is_ident("default") {
            Ok(Some(nv.value))
        } else {
            Err(syn::Error::new_spanned(
                attr,
                "expected 'default' in param attribute",
            ))
        }
    } else {
        Err(syn::Error::new_spanned(
            attr,
            "expected #[param(default = ...)]",
        ))
    }
}

fn parse_len(attr: &Attribute) -> syn::Result<Option<syn::Expr>> {
    let meta_list = match &attr.meta {
        Meta::List(list) => list,
        Meta::Path(_) => return Ok(None),
        _ => {
            return Err(syn::Error::new_spanned(
                attr,
                "expected #[input] or #[input(len = ...)]",
            ))
        }
    };

    if meta_list.tokens.is_empty() {
        return Ok(None);
    }

    let nested_meta: Meta = syn::parse2(meta_list.tokens.clone())?;
    if let Meta::NameValue(nv) = nested_meta {
        if nv.path.is_ident("len") {
            Ok(Some(nv.value))
        } else {
            Err(syn::Error::new_spanned(
                attr,
                "expected 'len' in input/output attribute",
            ))
        }
    } else {
        Err(syn::Error::new_spanned(
            attr,
            "expected #[input(len = ...)] or #[output(len = ...)]",
        ))
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
pub struct Model {
    docs: Docs,
    vis: Visibility,
    name: Ident,
    generics: syn::Generics, // Added generics field
    args: Vec<Arg>,
    body: Box<Block>,
}

impl Parse for Model {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut item = ItemFn::parse(input)?;
        //convert_impl_trait_to_generic(&mut item.sig);

        let docs = Docs::new(&item.attrs);

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
            vis: item.vis.clone(),
            name: convert_from_snake_case(&item.sig.ident),
            generics: item.sig.generics.clone(), // Extract generics
            args: props,
            body: item.block,
        })
    }
}

pub struct ArgsModel(pub ReactorArgs, pub Model);

impl ToTokens for ArgsModel {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let Self(
            reactor_args,
            Model {
                docs,
                vis,
                name,
                generics,
                args,
                body,
            },
        ) = self;

        // Extract generics parts
        let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

        // name of the generated Ports struct
        let ports_name = format_ident!("{name}Ports");

        let state_args = args
            .iter()
            .filter_map(
                |Arg {
                     docs,
                     kind,
                     name,
                     ty,
                 }| match kind {
                    ArgKind::State { default: _ } => Some(quote! { #docs pub #name: #ty }),
                    _ => None,
                },
            )
            .collect::<Vec<_>>();

        if !state_args.is_empty() && reactor_args.state.is_some() {
            abort!(
                reactor_args.state,
                "cannot use both #[reactor(state = ..)] and `#[state]` arguments at the same time."
            );
        }

        // name of the State struct
        let state_ident = format_ident!("{name}State");
        let state_type_path = if let Some(state) = &reactor_args.state {
            state.path.clone()
        } else if !state_args.is_empty() {
            parse_quote! { #state_ident #ty_generics }
        } else {
            // If there are no state args, then we exclude type generics here
            parse_quote! { #state_ident }
        };

        let port_args = args.iter().filter_map(
            |Arg {
                 docs,
                 kind,
                 name,
                 ty,
             }| match kind {
                ArgKind::Input { len } => Some(match len {
                    Some(len) => quote! { #[input(len = #len)] #docs pub #name: #ty },
                    None => quote! { #[input] #docs pub #name: #ty },
                }),
                ArgKind::Output { len } => Some(match len {
                    Some(len) => quote! { #[output(len = #len)] #docs pub #name: #ty },
                    None => quote! { #[output] #docs pub #name: #ty },
                }),
                _ => None,
            },
        );

        let port_struct = quote! {
            #[reactor_ports]
            #vis struct #ports_name #impl_generics #where_clause {
                #(#port_args),*
            }
        };

        // Default initialization for state fields
        let state_args_default = args
            .iter()
            .filter_map(|Arg { kind, name, .. }| match kind {
                ArgKind::State { default } => {
                    let val = default.clone().unwrap_or_else(|| {
                        parse_quote! {
                            ::core::default::Default::default()
                        }
                    });
                    Some(quote! { #name: #val })
                }
                _ => None,
            })
            .collect::<Vec<_>>();

        let state_struct = if reactor_args.state.is_none() {
            let state_struct = if state_args.is_empty() {
                quote! {
                    #vis type #state_ident = ();
                }
            } else {
                quote! {
                    #[derive(Clone)]
                    #vis struct #state_ident #impl_generics #where_clause {
                        #(#state_args),*
                    }
                }
            };
            Some(state_struct)
        } else {
            None
        };

        let state_impl = if reactor_args.state.is_none() && !state_args.is_empty() {
            Some(quote! {
                impl #impl_generics ::core::default::Default for #state_ident #ty_generics {
                    fn default() -> Self {
                        Self {
                            #(#state_args_default),*
                        }
                    }
                }
            })
        } else {
            None
        };

        let port_idents = args
            .iter()
            .filter_map(|Arg { kind, name, .. }| match *kind {
                ArgKind::Input { .. } | ArgKind::Output { .. } => Some(name),
                _ => None,
            });

        //TODO for now param args are just re-built into the output function signature. In the future, I may want to generate a builder type instead to support defaults.
        let param_args = args
            .iter()
            .filter_map(|Arg { kind, name, ty, .. }| match kind {
                ArgKind::Param { default: _default } => Some(quote! { #name: #ty }),
                _ => None,
            });

        let ret = quote! { -> impl ::boomerang::builder::Reactor<#state_type_path, Ports = #ports_name #ty_generics> };

        let output = quote! {
            #port_struct
            #state_struct
            #state_impl

            #[allow(non_snake_case)]
            #docs
            #vis fn #name #impl_generics(#(#param_args,)*) #ret #where_clause {
                <#ports_name #ty_generics as ::boomerang::builder::ReactorPorts>::build_with::<_, #state_type_path>(
                    move |builder, (#(#port_idents,)*)| {
                        #body
                        Ok(())
                    })
            }
        };

        tokens.append_all(output)
    }
}

#[test]
fn test() {
    let parsed: TypePath = parse_quote! {
        CountPorts<T>
    };
    dbg!(parsed);
}
