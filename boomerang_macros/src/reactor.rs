use proc_macro2::{Delimiter, Span, TokenStream, TokenTree};
use proc_macro_error2::abort;
use quote::{format_ident, quote, quote_spanned, ToTokens, TokenStreamExt};
use syn::{
    braced, parse::Parse, parse_quote, spanned::Spanned, Attribute, FnArg, Ident, Meta, Pat,
    PatIdent, Signature, Type, TypePath, Visibility,
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
struct ModeDecl {
    initial: bool,
    name: Ident,
    body: ReactorBody,
}

impl ModeDecl {
    fn key_ident(&self) -> Ident {
        format_ident!("__boomerang_mode_key_{}", self.name)
    }

    fn name_str(&self) -> String {
        self.name.to_string()
    }
}

#[derive(Debug)]
enum BodyItem {
    Tokens(TokenStream),
    Mode(ModeDecl),
}

#[derive(Debug)]
struct ReactorBody {
    items: Vec<BodyItem>,
}

impl ReactorBody {
    fn parse_tokens(tokens: TokenStream, allow_modes: bool) -> syn::Result<Self> {
        let tokens = tokens.into_iter().collect::<Vec<_>>();
        let mut items = Vec::new();
        let mut pending = TokenStream::new();
        let mut idx = 0;

        while idx < tokens.len() {
            match Self::parse_mode_at(&tokens, idx, allow_modes)? {
                Some((mode, consumed)) => {
                    if !pending.is_empty() {
                        items.push(BodyItem::Tokens(pending));
                        pending = TokenStream::new();
                    }
                    items.push(BodyItem::Mode(mode));
                    idx += consumed;
                }
                None => {
                    pending.extend(std::iter::once(tokens[idx].clone()));
                    idx += 1;
                }
            }
        }

        if !pending.is_empty() {
            items.push(BodyItem::Tokens(pending));
        }

        Ok(Self { items })
    }

    fn parse_mode_at(
        tokens: &[TokenTree],
        idx: usize,
        allow_modes: bool,
    ) -> syn::Result<Option<(ModeDecl, usize)>> {
        let Some(first) = tokens.get(idx) else {
            return Ok(None);
        };

        if is_ident(first, "mode")
            && tokens.get(idx + 1).is_some_and(is_bang)
            && matches!(tokens.get(idx + 2), Some(TokenTree::Group(group)) if group.delimiter() == Delimiter::Brace)
        {
            if !allow_modes {
                return Err(syn::Error::new_spanned(
                    first,
                    "nested mode blocks are not supported",
                ));
            }
            let Some(TokenTree::Group(group)) = tokens.get(idx + 2) else {
                unreachable!();
            };
            return Ok(Some((Self::parse_mode_macro(group)?, 3)));
        }

        Ok(None)
    }

    fn parse_mode_macro(group: &proc_macro2::Group) -> syn::Result<ModeDecl> {
        let tokens = group.stream().into_iter().collect::<Vec<_>>();
        let Some(first) = tokens.first() else {
            return Err(syn::Error::new_spanned(group, "expected mode declaration"));
        };

        let (initial, name_idx) = if is_ident(first, "initial") {
            (true, 1)
        } else {
            (false, 0)
        };

        let name = match tokens.get(name_idx) {
            Some(TokenTree::Ident(ident)) => ident.clone(),
            Some(other) => {
                return Err(syn::Error::new_spanned(
                    other,
                    "expected mode name in `mode!` block",
                ))
            }
            None => {
                return Err(syn::Error::new_spanned(
                    group,
                    "expected mode name in `mode!` block",
                ))
            }
        };

        let body_group = match tokens.get(name_idx + 1) {
            Some(TokenTree::Group(body_group)) if body_group.delimiter() == Delimiter::Brace => {
                body_group
            }
            Some(other) => {
                return Err(syn::Error::new_spanned(
                    other,
                    "expected `{ ... }` mode body",
                ))
            }
            None => {
                return Err(syn::Error::new_spanned(
                    group,
                    "expected `{ ... }` mode body",
                ))
            }
        };

        if let Some(extra) = tokens.get(name_idx + 2) {
            return Err(syn::Error::new_spanned(
                extra,
                "unexpected tokens after mode body",
            ));
        }

        Ok(ModeDecl {
            initial,
            name,
            body: ReactorBody::parse_tokens(body_group.stream(), false)?,
        })
    }

    fn mode_bindings(&self) -> Vec<TokenStream> {
        self.items
            .iter()
            .filter_map(|item| {
                let BodyItem::Mode(mode) = item else {
                    return None;
                };
                let key_ident = mode.key_ident();
                let effect_ident = &mode.name;
                let name = mode.name_str();
                let kind = if mode.initial {
                    quote!(::boomerang::builder::ModeKind::Initial)
                } else {
                    quote!(::boomerang::builder::ModeKind::Normal)
                };

                Some(quote! {
                    let #key_ident = builder.add_mode(#name, #kind)?;
                    #[allow(unused_variables)]
                    let #effect_ident = builder.reset_mode_effect(#key_ident)?;
                })
            })
            .collect()
    }

    fn body_tokens(&self) -> TokenStream {
        let mut tokens = TokenStream::new();
        for item in &self.items {
            match item {
                BodyItem::Tokens(body_tokens) => tokens.append_all(body_tokens.clone()),
                BodyItem::Mode(mode) => {
                    let key_ident = mode.key_ident();
                    let body = mode.body.body_tokens();
                    tokens.append_all(quote! {
                        builder.in_mode(#key_ident, |builder| {
                            #body
                            Ok(())
                        })?;
                    });
                }
            }
        }
        tokens
    }

    #[cfg(test)]
    fn modes(&self) -> impl Iterator<Item = &ModeDecl> {
        self.items.iter().filter_map(|item| match item {
            BodyItem::Mode(mode) => Some(mode),
            BodyItem::Tokens(_) => None,
        })
    }
}

fn is_ident(token: &TokenTree, expected: &str) -> bool {
    matches!(token, TokenTree::Ident(ident) if ident == expected)
}

fn is_bang(token: &TokenTree) -> bool {
    matches!(token, TokenTree::Punct(punct) if punct.as_char() == '!')
}

#[derive(Debug)]
pub struct Model {
    docs: Docs,
    vis: Visibility,
    name: Ident,
    generics: syn::Generics, // Added generics field
    args: Vec<Arg>,
    body: ReactorBody,
}

impl Parse for Model {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let attrs = Attribute::parse_outer(input)?;
        let vis = input.parse::<Visibility>()?;
        let sig = input.parse::<Signature>()?;
        let content;
        braced!(content in input);
        let body_tokens = content.parse::<TokenStream>()?;
        let body = ReactorBody::parse_tokens(body_tokens, true)?;

        if !input.is_empty() {
            return Err(input.error("unexpected tokens after reactor function body"));
        }

        let docs = Docs::new(&attrs);

        let props = sig
            .inputs
            .clone()
            .into_iter()
            .map(Arg::from)
            .collect::<Vec<_>>();

        Ok(Self {
            docs,
            vis,
            name: convert_from_snake_case(&sig.ident),
            generics: sig.generics.clone(), // Extract generics
            args: props,
            body,
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

        let has_banked_ports = args.iter().any(|Arg { kind, .. }| {
            matches!(
                kind,
                ArgKind::Input { len: Some(_) } | ArgKind::Output { len: Some(_) }
            )
        });
        let mode_bindings = body.mode_bindings();
        let body_tokens = body.body_tokens();

        let output = if has_banked_ports {
            let ports_struct_fields = args.iter().filter_map(
                |Arg {
                     docs,
                     kind,
                     name,
                     ty,
                 }| match kind {
                    ArgKind::Input { len } | ArgKind::Output { len } => {
                        let dir = match kind {
                            ArgKind::Input { .. } => quote!(::boomerang::builder::Input),
                            ArgKind::Output { .. } => quote!(::boomerang::builder::Output),
                            _ => unreachable!(),
                        };
                        let field_ty = match ty {
                            syn::Type::Array(array) => {
                                if len.is_some() {
                                    abort!(ty, "banked ports cannot be declared as arrays");
                                }
                                let element_type = &array.elem;
                                let len_expr = &array.len;
                                quote!([::boomerang::builder::TypedPortKey<#element_type, #dir, ::boomerang::builder::Contained>; #len_expr])
                            }
                            _ if len.is_some() => {
                                quote!(::boomerang::builder::PortBank<#ty, #dir, ::boomerang::builder::Contained>)
                            }
                            _ => {
                                quote!(::boomerang::builder::TypedPortKey<#ty, #dir, ::boomerang::builder::Contained>)
                            }
                        };

                        Some(quote! { #docs pub #name: #field_ty })
                    }
                    _ => None,
                },
            );

            let ports_struct = quote! {
                #vis struct #ports_name #impl_generics #where_clause {
                    #(#ports_struct_fields,)*
                }
            };

            let len_bindings = args.iter().filter_map(|Arg { kind, name, .. }| match kind {
                ArgKind::Input { len: Some(expr) } | ArgKind::Output { len: Some(expr) } => {
                    let len_name = format_ident!("{}_len", name.ident);
                    Some(quote! { let #len_name = #expr; })
                }
                _ => None,
            });

            let local_patterns: Vec<_> = args
                .iter()
                .filter_map(|Arg { kind, name, .. }| match kind {
                    ArgKind::Input { .. } | ArgKind::Output { .. } => Some(name.ident.clone()),
                    _ => None,
                })
                .collect();

            let local_values: Vec<_> = args
                .iter()
                .filter_map(|Arg { kind, name, .. }| match kind {
                    ArgKind::Input { len: Some(_) } | ArgKind::Output { len: Some(_) } => {
                        Some(format_ident!("{}_for_fn", name.ident))
                    }
                    ArgKind::Input { len: None } | ArgKind::Output { len: None } => {
                        Some(name.ident.clone())
                    }
                    _ => None,
                })
                .collect();

            let local_types: Vec<_> = args
                .iter()
                .filter_map(|Arg { kind, ty, .. }| match kind {
                    ArgKind::Input { len } | ArgKind::Output { len } => {
                        let dir = match kind {
                            ArgKind::Input { .. } => quote!(::boomerang::builder::Input),
                            ArgKind::Output { .. } => quote!(::boomerang::builder::Output),
                            _ => unreachable!(),
                        };
                        let local_ty = match ty {
                            syn::Type::Array(array) => {
                                if len.is_some() {
                                    abort!(ty, "banked ports cannot be declared as arrays");
                                }
                                let element_type = &array.elem;
                                let len_expr = &array.len;
                                quote!([::boomerang::builder::TypedPortKey<#element_type, #dir>; #len_expr])
                            }
                            _ if len.is_some() => {
                                quote!(::boomerang::builder::PortBank<#ty, #dir>)
                            }
                            _ => {
                                quote!(::boomerang::builder::TypedPortKey<#ty, #dir>)
                            }
                        };
                        Some(local_ty)
                    }
                    _ => None,
                })
                .collect();

            let create_ports = args.iter().filter_map(|Arg { kind, name, ty, .. }| match kind {
                ArgKind::Input { len } | ArgKind::Output { len } => {
                    let name_str = name.ident.to_string();
                    let dir = match kind {
                        ArgKind::Input { .. } => quote!(::boomerang::builder::Input),
                        ArgKind::Output { .. } => quote!(::boomerang::builder::Output),
                        _ => unreachable!(),
                    };
                    let for_fn_name = format_ident!("{}_for_fn", name.ident);

                    match ty {
                        syn::Type::Array(array) => {
                            if len.is_some() {
                                abort!(ty, "banked ports cannot be declared as arrays");
                            }
                            let element_type = &array.elem;
                            let len_expr = &array.len;
                            match kind {
                                ArgKind::Input { .. } => Some(quote! {
                                    let #name = builder.add_input_ports::<#element_type, #len_expr>(#name_str)?;
                                }),
                                ArgKind::Output { .. } => Some(quote! {
                                    let #name = builder.add_output_ports::<#element_type, #len_expr>(#name_str)?;
                                }),
                                _ => None,
                            }
                        }
                        _ => match (kind, len) {
                            (ArgKind::Input { .. }, Some(_)) => {
                                let len_name = format_ident!("{}_len", name.ident);
                                Some(quote! {
                                    let #name = builder.add_input_bank::<#ty>(#name_str, #len_name)?;
                                    let #for_fn_name = #name.clone();
                                })
                            }
                            (ArgKind::Output { .. }, Some(_)) => {
                                let len_name = format_ident!("{}_len", name.ident);
                                Some(quote! {
                                    let #name = builder.add_output_bank::<#ty>(#name_str, #len_name)?;
                                    let #for_fn_name = #name.clone();
                                })
                            }
                            _ => Some(quote! {
                                let #name = builder.add_port::<#ty, #dir>(#name_str, None)?;
                            }),
                        },
                    }
                }
                _ => None,
            });

            let field_inits = args
                .iter()
                .filter_map(|Arg { kind, name, ty, .. }| match kind {
                    ArgKind::Input { .. } | ArgKind::Output { .. } => match ty {
                        syn::Type::Array(_) => Some(quote! {
                            #name: std::array::from_fn(|i| #name[i].contained())
                        }),
                        _ => Some(quote!(#name: #name.contained())),
                    },
                    _ => None,
                });

            quote! {
                #ports_struct
                #state_struct
                #state_impl

                #[allow(non_snake_case)]
                #docs
                #vis fn #name #impl_generics(#(#param_args,)*) #ret #where_clause {
                    move |name: &str,
                         state: #state_type_path,
                         parent: Option<::boomerang::builder::BuilderReactorKey>,
                         scope_mode: Option<::boomerang::builder::BuilderModeKey>,
                         bank_info: Option<::boomerang::runtime::BankInfo>,
                         placement: ::boomerang::builder::ReactorPlacement,
                         env: &mut ::boomerang::builder::Assembly| {
                        #(#len_bindings)*
                        let mut builder = env.add_reactor(name, parent, bank_info, state, placement);
                        if let Some(scope_mode) = scope_mode {
                            builder.set_scope_mode(scope_mode)?;
                        }
                        #(#create_ports)*
                        (move |builder: &mut ::boomerang::builder::ReactorBuilderState<'_, #state_type_path>,
                              ports: (#(#local_types,)* )| -> Result<(), ::boomerang::builder::BuilderError> {
                            #[allow(non_snake_case)]
                            let (#(#local_patterns,)*) = ports;
                            #(#mode_bindings)*
                            #body_tokens
                            Ok(())
                        })(&mut builder, (#(#local_values,)*))?;
                        builder.finish()?;
                        Ok(#ports_name {
                            #(#field_inits,)*
                        })
                    }
                }
            }
        } else {
            quote! {
                #port_struct
                #state_struct
                #state_impl

                #[allow(non_snake_case)]
                #docs
                #vis fn #name #impl_generics(#(#param_args,)*) #ret #where_clause {
                    <#ports_name #ty_generics as ::boomerang::builder::ReactorPorts>::build_with::<_, #state_type_path>(
                        move |builder, (#(#port_idents,)*)| {
                            #(#mode_bindings)*
                            #body_tokens
                            Ok(())
                        })
                }
            }
        };

        tokens.append_all(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_structural_mode_blocks() {
        let model = syn::parse_str::<Model>(
            r#"
            fn Example() -> impl Reactor {
                let root = 1;
                mode! { initial idle {
                    reaction! {
                        (startup) -> active {
                            active.set(ctx);
                        }
                    }
                } }
                mode! { active {
                    reaction! {
                        (reset) -> history(idle) {
                            idle.set(ctx);
                        }
                    }
                } }
                let after = root + 1;
            }
            "#,
        )
        .unwrap();

        let modes = model.body.modes().collect::<Vec<_>>();
        assert_eq!(modes.len(), 2);
        assert!(modes[0].initial);
        assert_eq!(modes[0].name, "idle");
        assert!(!modes[1].initial);
        assert_eq!(modes[1].name, "active");
    }

    #[test]
    fn rejects_direct_nested_mode_blocks() {
        let err = syn::parse_str::<Model>(
            r#"
            fn Example() -> impl Reactor {
                mode! { initial idle {
                    mode! { active {
                    } }
                } }
            }
            "#,
        )
        .unwrap_err();

        assert!(err.to_string().contains("nested mode blocks"));
    }
}
