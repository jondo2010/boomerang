use proc_macro2::{Delimiter, Span, TokenStream, TokenTree};
use proc_macro_error2::abort;
use quote::{format_ident, quote, quote_spanned, ToTokens, TokenStreamExt};
use std::collections::BTreeSet;
use syn::{
    braced, ext::IdentExt, parse::Parse, parse_quote, spanned::Spanned, Attribute, FnArg, Ident,
    Meta, Pat, PatIdent, Signature, Type, TypePath, Visibility,
};

use crate::util::convert_from_snake_case;

/// Top-level arguments for the #[reactor] macro
#[derive(attribute_derive::FromAttr)]
pub struct ReactorArgs {
    /// The name of the state type to use. If not provided, a state struct will be generated.
    state: Option<TypePath>,
    /// Zero-argument constructor for custom state in payload builds.
    state_init: Option<syn::Path>,
    /// Stable component contract identity for descriptor builds.
    contract: Option<syn::LitStr>,
    /// Stable component contract version for descriptor builds.
    contract_version: Option<syn::LitInt>,
}

impl ReactorArgs {
    fn validate(&self) -> syn::Result<()> {
        if let (Some(state_init), None) = (&self.state_init, &self.state) {
            return Err(syn::Error::new_spanned(
                state_init,
                "`state_init` requires `state = T`",
            ));
        }
        Ok(())
    }
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
        format_ident!("__boomerang_mode_key_{}", ident_text(&self.name))
    }

    fn name_str(&self) -> String {
        ident_text(&self.name)
    }
}

#[derive(Debug)]
enum BodyItem {
    Tokens(TokenStream),
    Mode(ModeDecl),
    Reaction(crate::reaction::Model),
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
                    if let Some((reaction, consumed)) = Self::parse_reaction_at(&tokens, idx)? {
                        if !pending.is_empty() {
                            items.push(BodyItem::Tokens(pending));
                            pending = TokenStream::new();
                        }
                        items.push(BodyItem::Reaction(reaction));
                        idx += consumed;
                    } else {
                        pending.extend(std::iter::once(tokens[idx].clone()));
                        idx += 1;
                    }
                }
            }
        }

        if !pending.is_empty() {
            items.push(BodyItem::Tokens(pending));
        }

        Ok(Self { items })
    }

    fn parse_reaction_at(
        tokens: &[TokenTree],
        idx: usize,
    ) -> syn::Result<Option<(crate::reaction::Model, usize)>> {
        if !tokens
            .get(idx)
            .is_some_and(|token| is_ident(token, "reaction"))
            || !tokens.get(idx + 1).is_some_and(is_bang)
        {
            return Ok(None);
        }
        let Some(TokenTree::Group(group)) = tokens.get(idx + 2) else {
            return Ok(None);
        };
        let reaction = syn::parse2::<crate::reaction::Model>(group.stream())?;
        let consumed = if tokens.get(idx + 3).is_some_and(is_semicolon) {
            4
        } else {
            3
        };
        Ok(Some((reaction, consumed)))
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
                    let #key_ident = ctx.add_mode(#name, #kind)?;
                    #[allow(unused_variables)]
                    let #effect_ident = ctx.reset_mode_effect(#key_ident)?;
                })
            })
            .collect()
    }

    fn body_tokens(&self) -> TokenStream {
        let mut tokens = TokenStream::new();
        for item in &self.items {
            match item {
                BodyItem::Tokens(body_tokens) => tokens.append_all(body_tokens.clone()),
                BodyItem::Reaction(reaction) => reaction.to_tokens(&mut tokens),
                BodyItem::Mode(mode) => {
                    let key_ident = mode.key_ident();
                    let body = mode.body.body_tokens();
                    tokens.append_all(quote! {
                        ctx.in_mode(#key_ident, |ctx| {
                            #body
                            Ok(())
                        })?;
                    });
                }
            }
        }
        tokens
    }

    fn unsupported_span(&self) -> Option<Span> {
        self.items.iter().find_map(|item| match item {
            BodyItem::Tokens(tokens) => tokens.clone().into_iter().next().map(|token| token.span()),
            BodyItem::Mode(mode) => mode.body.unsupported_span(),
            BodyItem::Reaction(_) => None,
        })
    }

    fn reactions<'a>(
        &'a self,
        scope: Option<&'a ModeDecl>,
        output: &mut Vec<(Option<&'a ModeDecl>, &'a crate::reaction::Model)>,
    ) {
        for item in &self.items {
            match item {
                BodyItem::Mode(mode) => mode.body.reactions(Some(mode), output),
                BodyItem::Reaction(reaction) => output.push((scope, reaction)),
                BodyItem::Tokens(_) => {}
            }
        }
    }

    fn validate_unique_reaction_names(&self) -> syn::Result<()> {
        let mut reactions = Vec::new();
        self.reactions(None, &mut reactions);
        let mut names = BTreeSet::new();
        for (_, reaction) in reactions {
            if let Some(name) = reaction.name() {
                if !names.insert(ident_text(name)) {
                    return Err(syn::Error::new(
                        name.span(),
                        format!("duplicate reaction name `{name}`"),
                    ));
                }
            }
        }
        Ok(())
    }

    #[cfg(test)]
    fn modes(&self) -> impl Iterator<Item = &ModeDecl> {
        self.items.iter().filter_map(|item| match item {
            BodyItem::Mode(mode) => Some(mode),
            BodyItem::Tokens(_) | BodyItem::Reaction(_) => None,
        })
    }
}

fn is_ident(token: &TokenTree, expected: &str) -> bool {
    matches!(token, TokenTree::Ident(ident) if ident == expected)
}

fn is_bang(token: &TokenTree) -> bool {
    matches!(token, TokenTree::Punct(punct) if punct.as_char() == '!')
}

fn is_semicolon(token: &TokenTree) -> bool {
    matches!(token, TokenTree::Punct(punct) if punct.as_char() == ';')
}

fn ident_text(ident: &Ident) -> String {
    ident.unraw().to_string()
}

fn path_segments(path: &crate::reaction::PathOrIdent) -> Vec<String> {
    match path {
        crate::reaction::PathOrIdent::Simple(ident) => vec![ident_text(ident)],
        crate::reaction::PathOrIdent::Field(field) => {
            let mut segments = match field.base.as_ref() {
                syn::Expr::Path(path) => path
                    .path
                    .segments
                    .iter()
                    .map(|segment| ident_text(&segment.ident))
                    .collect::<Vec<_>>(),
                _ => unreachable!("PathOrIdent fields have an identifier base"),
            };
            segments.push(match &field.member {
                syn::Member::Named(ident) => ident_text(ident),
                syn::Member::Unnamed(index) => index.index.to_string(),
            });
            segments
        }
    }
}

fn stable_path_tokens(segments: &[String]) -> TokenStream {
    let (first, rest) = segments.split_first().expect("stable paths are non-empty");
    quote! {{
        let path = ::boomerang::builder::compiler::StablePath::from_name(#first)
            .expect("macro generated a valid stable path");
        #(let path = path.append_name(#rest)
            .expect("macro generated a valid stable path");)*
        path
    }}
}

fn relationship_target(
    reactor_name: &str,
    port_names: &[String],
    mode_names: &[String],
    path: &crate::reaction::PathOrIdent,
) -> (TokenStream, bool) {
    let path = path_segments(path);
    let leaf = path.first().expect("paths are non-empty");
    let mut slot_segments = vec![reactor_name.to_owned()];
    slot_segments.extend(path.iter().cloned());
    let slot = stable_path_tokens(&slot_segments);
    if path.len() == 1 && mode_names.contains(leaf) {
        (
            quote! {
                ::boomerang::builder::DescriptorRelationshipTarget::Mode(
                    ::boomerang::builder::ModeSlotId::from_path(#slot),
                )
            },
            true,
        )
    } else if path.len() == 1 && port_names.contains(leaf) {
        (
            quote! {
                ::boomerang::builder::DescriptorRelationshipTarget::Port(
                    ::boomerang::builder::PortSlotId::from_path(#slot),
                )
            },
            false,
        )
    } else {
        (
            quote! {
                ::boomerang::builder::DescriptorRelationshipTarget::Lexical(
                    #slot,
                )
            },
            false,
        )
    }
}

/// Builds a compiler stable path from macro-normalized name segments.
fn stable_path_value(segments: &[String]) -> boomerang_builder::compiler::StablePath {
    let (first, rest) = segments.split_first().expect("stable paths are non-empty");
    let mut path = boomerang_builder::compiler::StablePath::from_name(first.clone())
        .expect("macro generated a valid stable path");
    for segment in rest {
        path = path
            .append_name(segment.clone())
            .expect("macro generated a valid stable path");
    }
    path
}

/// Resolves a reaction reference and reports whether it denotes a mode.
fn payload_relationship_target(
    reactor_name: &str,
    port_names: &[String],
    mode_names: &[String],
    path: &crate::reaction::PathOrIdent,
) -> (boomerang_builder::DescriptorRelationshipTarget, bool) {
    let path = path_segments(path);
    let leaf = path.first().expect("paths are non-empty").clone();
    let mut slot_segments = vec![reactor_name.to_owned()];
    slot_segments.extend(path);
    let slot = stable_path_value(&slot_segments);
    if slot_segments.len() == 2 && mode_names.contains(&leaf) {
        (
            boomerang_builder::DescriptorRelationshipTarget::Mode(
                boomerang_builder::ModeSlotId::from_path(slot),
            ),
            true,
        )
    } else if slot_segments.len() == 2 && port_names.contains(&leaf) {
        (
            boomerang_builder::DescriptorRelationshipTarget::Port(
                boomerang_builder::PortSlotId::from_path(slot),
            ),
            false,
        )
    } else {
        (
            boomerang_builder::DescriptorRelationshipTarget::Lexical(slot),
            false,
        )
    }
}

fn required_binding_symbol(reactor: &str, reaction: Option<&str>) -> Ident {
    let component = boomerang_builder::compiler::ComponentInstanceId::new("macro")
        .expect("macro is a valid component ID");
    let implementation = boomerang_builder::compiler::ImplementationId::new("macro")
        .expect("macro is a valid implementation ID");
    let reactor_path = stable_path_value(std::slice::from_ref(&reactor.to_owned()));
    let binding = if let Some(reaction) = reaction {
        let reaction_path = if let Some(ordinal) = reaction.strip_prefix("#g") {
            reactor_path.append_generated_ordinal(
                ordinal
                    .parse()
                    .expect("macro generated a numeric reaction ordinal"),
            )
        } else {
            reactor_path
                .append_name(reaction)
                .expect("macro generated a valid reaction name")
        };
        boomerang_builder::compiler::RequiredBinding::Reaction {
            component,
            implementation,
            reaction: boomerang_builder::ReactionSlotId::from_path(reaction_path),
        }
    } else {
        boomerang_builder::compiler::RequiredBinding::State {
            component,
            implementation,
            reactor: boomerang_builder::ReactorSlotId::from_path(reactor_path),
        }
    };
    format_ident!("{}", binding.symbol())
}

fn payload_ref(
    reactor_name: &str,
    args: &[Arg],
    mode_names: &[String],
    path: &crate::reaction::PathOrIdent,
) -> syn::Result<(Ident, TokenStream)> {
    let crate::reaction::PathOrIdent::Simple(ident) = path else {
        return Err(syn::Error::new_spanned(
            path,
            "payload mode supports only own ports, modes, and lifecycle relations",
        ));
    };
    let name = ident_text(ident);
    if mode_names.contains(&name) {
        return Ok((ident.clone(), quote!(::boomerang::runtime::ModeEffectRef)));
    }
    let Some(arg) = args.iter().find(|arg| ident_text(&arg.name.ident) == name) else {
        return Err(syn::Error::new_spanned(
            path,
            format!(
                "payload mode supports only own ports, modes, and lifecycle relations; `{reactor_name}/{name}` is lexical"
            ),
        ));
    };
    let reference = match (&arg.kind, &arg.ty) {
        (ArgKind::Input { len: None }, Type::Array(array)) => {
            let element = &array.elem;
            let len = &array.len;
            quote!([::boomerang::runtime::InputRef<'store, #element>; #len])
        }
        (ArgKind::Output { len: None }, Type::Array(array)) => {
            let element = &array.elem;
            let len = &array.len;
            quote!([::boomerang::runtime::OutputRef<'store, #element>; #len])
        }
        (ArgKind::Input { len: Some(_) }, ty) => {
            quote!(::boomerang::runtime::InputBankRef<'store, #ty>)
        }
        (ArgKind::Output { len: Some(_) }, ty) => {
            quote!(::boomerang::runtime::OutputBankRef<'store, #ty>)
        }
        (ArgKind::Input { len: None }, ty) => {
            quote!(::boomerang::runtime::InputRef<'store, #ty>)
        }
        (ArgKind::Output { len: None }, ty) => {
            quote!(::boomerang::runtime::OutputRef<'store, #ty>)
        }
        (ArgKind::State { .. } | ArgKind::Param { .. }, _) => {
            return Err(syn::Error::new_spanned(
                path,
                "payload mode supports only own ports, modes, and lifecycle relations",
            ))
        }
    };
    Ok((ident.clone(), reference))
}

fn payload_reaction_output(
    model: &Model,
    state_type: &syn::Path,
    mode_names: &[String],
) -> syn::Result<Vec<TokenStream>> {
    let reactor_name = ident_text(&model.name);
    let mut reactions = Vec::new();
    model.body.reactions(None, &mut reactions);
    reactions
        .into_iter()
        .enumerate()
        .map(|(ordinal, (_, reaction))| {
            let reaction_name = reaction
                .name()
                .map(ident_text)
                .unwrap_or_else(|| format!("#g{ordinal}"));
            let symbol = required_binding_symbol(&reactor_name, Some(&reaction_name));
            let mut refs = Vec::new();
            for trigger in reaction.triggers() {
                match trigger {
                    crate::reaction::TriggerType::Startup => refs.push((
                        format_ident!("startup"),
                        quote!(::boomerang::runtime::ActionRef<'store>),
                    )),
                    crate::reaction::TriggerType::Shutdown => refs.push((
                        format_ident!("shutdown"),
                        quote!(::boomerang::runtime::ActionRef<'store>),
                    )),
                    crate::reaction::TriggerType::Reset => {}
                    crate::reaction::TriggerType::Regular(path) => {
                        refs.push(payload_ref(&reactor_name, &model.args, mode_names, path)?);
                    }
                }
            }
            for path in reaction.uses() {
                refs.push(payload_ref(&reactor_name, &model.args, mode_names, path)?);
            }
            for effect in reaction.effects() {
                let path = match effect {
                    crate::reaction::EffectType::Regular(path)
                    | crate::reaction::EffectType::Reset(path)
                    | crate::reaction::EffectType::History(path) => path,
                };
                refs.push(payload_ref(&reactor_name, &model.args, mode_names, path)?);
            }
            let (ref_names, ref_types): (Vec<_>, Vec<_>) = refs.into_iter().unzip();
            let code = reaction.code();
            let mut generics = model.generics.clone();
            generics.params.insert(0, parse_quote!('store));
            let (impl_generics, _, where_clause) = generics.split_for_impl();
            Ok(quote! {
                #[allow(non_snake_case, unused_mut, unused_variables)]
                pub fn #symbol #impl_generics(
                    ctx: &mut ::boomerang::runtime::Context,
                    state: &mut #state_type,
                    (#(mut #ref_names,)*): (#(#ref_types,)*),
                ) #where_clause #code
            })
        })
        .collect()
}

/// Generates the payload facet's target-safe compatibility module.
fn payload_output(
    reactor_args: &ReactorArgs,
    model: &Model,
    state_type: &syn::Path,
    state_struct: &Option<TokenStream>,
    state_impl: &Option<TokenStream>,
) -> TokenStream {
    if reactor_args.contract.is_none() && reactor_args.contract_version.is_none() {
        return TokenStream::new();
    }
    let (Some(contract), Some(contract_version)) =
        (&reactor_args.contract, &reactor_args.contract_version)
    else {
        return quote! {
            compile_error!("deployment descriptor requires contract and contract_version metadata");
        };
    };
    if reactor_args.state.is_some() && reactor_args.state_init.is_none() {
        return quote! {
            compile_error!("payload mode requires `state_init = path` with `state = T`");
        };
    }

    let contract_text = contract.value();
    if contract_text.is_empty()
        || contract_text.trim() != contract_text
        || contract_text.chars().any(char::is_control)
    {
        return syn::Error::new(
            contract.span(),
            "contract must be non-empty, contain no control characters, and have no surrounding whitespace",
        )
        .to_compile_error();
    }
    let contract_version = match contract_version.base10_parse::<u64>() {
        Ok(contract_version) => contract_version,
        Err(_) => {
            return syn::Error::new(contract_version.span(), "contract_version must fit in u64")
                .to_compile_error()
        }
    };

    if let Err(error) = model.body.validate_unique_reaction_names() {
        return error.to_compile_error();
    }

    if let Some(span) = model.body.unsupported_span() {
        let diagnostic = quote_spanned! {span =>
            compile_error!("deployment descriptor requires reaction! syntax");
        };
        return quote! {
            #diagnostic
        };
    }

    let reactor_name = ident_text(&model.name);
    let reactor_path = stable_path_value(std::slice::from_ref(&reactor_name));
    let reactor_slot = boomerang_builder::ReactorSlot {
        id: boomerang_builder::ReactorSlotId::from_path(reactor_path.clone()),
        parent: None,
    };
    let port_names = model
        .args
        .iter()
        .filter_map(|arg| match arg.kind {
            ArgKind::Input { .. } | ArgKind::Output { .. } => Some(ident_text(&arg.name.ident)),
            ArgKind::State { .. } | ArgKind::Param { .. } => None,
        })
        .collect::<Vec<_>>();
    let port_slots = model
        .args
        .iter()
        .filter_map(|arg| {
            let direction = match arg.kind {
                ArgKind::Input { .. } => boomerang_builder::PortDirection::Input,
                ArgKind::Output { .. } => boomerang_builder::PortDirection::Output,
                ArgKind::State { .. } | ArgKind::Param { .. } => return None,
            };
            let path = stable_path_value(&[reactor_name.clone(), ident_text(&arg.name.ident)]);
            Some(boomerang_builder::PortSlot {
                id: boomerang_builder::PortSlotId::from_path(path),
                reactor: boomerang_builder::ReactorSlotId::from_path(reactor_path.clone()),
                direction,
            })
        })
        .collect::<Vec<_>>();
    let state_slots = model
        .args
        .iter()
        .filter(|arg| matches!(arg.kind, ArgKind::State { .. }))
        .map(|arg| boomerang_builder::StateSlot {
            id: boomerang_builder::StateSlotId::from_path(stable_path_value(&[
                reactor_name.clone(),
                ident_text(&arg.name.ident),
            ])),
            reactor: boomerang_builder::ReactorSlotId::from_path(reactor_path.clone()),
        })
        .collect::<Vec<_>>();

    let modes = model
        .body
        .items
        .iter()
        .filter_map(|item| match item {
            BodyItem::Mode(mode) => Some(mode),
            BodyItem::Tokens(_) | BodyItem::Reaction(_) => None,
        })
        .collect::<Vec<_>>();
    let mode_names = modes
        .iter()
        .map(|mode| ident_text(&mode.name))
        .collect::<Vec<_>>();
    let reaction_exports = match payload_reaction_output(model, state_type, &mode_names) {
        Ok(exports) => exports,
        Err(error) => return error.to_compile_error(),
    };
    let mode_slots = modes
        .iter()
        .map(|mode| boomerang_builder::ModeSlot {
            id: boomerang_builder::ModeSlotId::from_path(stable_path_value(&[
                reactor_name.clone(),
                ident_text(&mode.name),
            ])),
            reactor: boomerang_builder::ReactorSlotId::from_path(reactor_path.clone()),
            parent: None,
            initial: mode.initial,
        })
        .collect::<Vec<_>>();

    let mut reactions = Vec::new();
    model.body.reactions(None, &mut reactions);
    let mut reaction_slots = Vec::new();
    let mut relationships = Vec::new();
    for (ordinal, (scope, reaction)) in reactions.into_iter().enumerate() {
        let reaction_id = if let Some(name) = reaction.name() {
            boomerang_builder::ReactionSlotId::from_path(stable_path_value(&[
                reactor_name.clone(),
                ident_text(name),
            ]))
        } else {
            let ordinal =
                u32::try_from(ordinal).expect("reaction count exceeds descriptor representation");
            boomerang_builder::ReactionSlotId::from_path(
                reactor_path.append_generated_ordinal(ordinal),
            )
        };
        reaction_slots.push(boomerang_builder::ReactionSlot {
            id: reaction_id.clone(),
            reactor: boomerang_builder::ReactorSlotId::from_path(reactor_path.clone()),
        });

        for (position, trigger) in reaction.triggers().iter().enumerate() {
            let target = match trigger {
                crate::reaction::TriggerType::Startup => {
                    boomerang_builder::DescriptorRelationshipTarget::Lifecycle(
                        boomerang_builder::DescriptorLifecycle::Startup,
                    )
                }
                crate::reaction::TriggerType::Shutdown => {
                    boomerang_builder::DescriptorRelationshipTarget::Lifecycle(
                        boomerang_builder::DescriptorLifecycle::Shutdown,
                    )
                }
                crate::reaction::TriggerType::Reset => {
                    boomerang_builder::DescriptorRelationshipTarget::Lifecycle(
                        boomerang_builder::DescriptorLifecycle::Reset,
                    )
                }
                crate::reaction::TriggerType::Regular(path) => {
                    payload_relationship_target(&reactor_name, &port_names, &mode_names, path).0
                }
            };
            relationships.push(boomerang_builder::DescriptorRelationship {
                reaction: reaction_id.clone(),
                kind: boomerang_builder::DescriptorRelationshipKind::Trigger,
                target,
                mode_transition: None,
                declaration_position: u32::try_from(position)
                    .expect("reaction relationship count exceeds descriptor representation"),
            });
        }
        for (position, path) in reaction.uses().iter().enumerate() {
            relationships.push(boomerang_builder::DescriptorRelationship {
                reaction: reaction_id.clone(),
                kind: boomerang_builder::DescriptorRelationshipKind::Use,
                target: payload_relationship_target(&reactor_name, &port_names, &mode_names, path)
                    .0,
                mode_transition: None,
                declaration_position: u32::try_from(position)
                    .expect("reaction relationship count exceeds descriptor representation"),
            });
        }
        for (position, effect) in reaction.effects().iter().enumerate() {
            let path = match effect {
                crate::reaction::EffectType::Regular(path)
                | crate::reaction::EffectType::Reset(path)
                | crate::reaction::EffectType::History(path) => path,
            };
            let (target, is_mode) =
                payload_relationship_target(&reactor_name, &port_names, &mode_names, path);
            let is_transition = matches!(
                effect,
                crate::reaction::EffectType::Reset(_) | crate::reaction::EffectType::History(_)
            );
            let kind = if is_mode || is_transition {
                boomerang_builder::DescriptorRelationshipKind::Mode
            } else {
                boomerang_builder::DescriptorRelationshipKind::Effect
            };
            let mode_transition = match effect {
                crate::reaction::EffectType::History(_) => {
                    Some(boomerang_builder::ModeTransitionKind::History)
                }
                crate::reaction::EffectType::Reset(_) => {
                    Some(boomerang_builder::ModeTransitionKind::Reset)
                }
                crate::reaction::EffectType::Regular(_) if is_mode => {
                    Some(boomerang_builder::ModeTransitionKind::Reset)
                }
                crate::reaction::EffectType::Regular(_) => None,
            };
            relationships.push(boomerang_builder::DescriptorRelationship {
                reaction: reaction_id.clone(),
                kind,
                target,
                mode_transition,
                declaration_position: u32::try_from(position)
                    .expect("reaction relationship count exceeds descriptor representation"),
            });
        }
        if let Some(scope) = scope {
            relationships.push(boomerang_builder::DescriptorRelationship {
                reaction: reaction_id,
                kind: boomerang_builder::DescriptorRelationshipKind::Scope,
                target: boomerang_builder::DescriptorRelationshipTarget::Mode(
                    boomerang_builder::ModeSlotId::from_path(stable_path_value(&[
                        reactor_name.clone(),
                        ident_text(&scope.name),
                    ])),
                ),
                mode_transition: None,
                declaration_position: 0,
            });
        }
    }

    let descriptor = match boomerang_builder::ComponentDescriptor::try_new(
        boomerang_builder::compiler::ContractId::new(contract_text)
            .expect("reactor macro validated contract text"),
        contract_version,
        boomerang_builder::COMPONENT_DESCRIPTOR_MACRO_ABI,
        vec![reactor_slot],
        port_slots,
        vec![],
        reaction_slots,
        mode_slots,
        state_slots,
        vec![],
        relationships,
        vec![],
        vec![],
        boomerang_builder::DescriptorBounds::default(),
    ) {
        Ok(descriptor) => descriptor,
        Err(error) => {
            return syn::Error::new(
                contract.span(),
                format!("deployment payload descriptor is invalid: {error}"),
            )
            .to_compile_error()
        }
    };
    let fingerprint = descriptor
        .descriptor_fingerprint_input()
        .fingerprint()
        .to_bytes();
    let macro_abi = boomerang_builder::COMPONENT_DESCRIPTOR_MACRO_ABI;
    let state_symbol = required_binding_symbol(&reactor_name, None);
    let state_init = reactor_args.state_init.as_ref().map_or_else(
        || quote!(::core::default::Default::default()),
        |path| quote!(#path()),
    );
    let (state_generics, _, state_where_clause) = model.generics.split_for_impl();

    quote! {
        #state_struct
        #state_impl

        pub mod __boomerang {
            #[allow(unused_imports)]
            use super::*;

            /// Compatibility header for this payload facet.
            pub const BINDING_MANIFEST: ::boomerang::runtime::binding::BindingManifest =
                ::boomerang::runtime::binding::BindingManifest::new(
                    ::boomerang::runtime::binding::DescriptorFingerprint::new([#(#fingerprint),*]),
                    #macro_abi,
                );

            /// Constructs this reactor's concrete payload state.
            #[allow(non_snake_case)]
            pub fn #state_symbol #state_generics() -> #state_type #state_where_clause {
                #state_init
            }

            #(#reaction_exports)*
        }
    }
}

fn descriptor_output(reactor_args: &ReactorArgs, model: &Model) -> TokenStream {
    if reactor_args.contract.is_none() && reactor_args.contract_version.is_none() {
        return TokenStream::new();
    }
    let (Some(contract), Some(contract_version)) =
        (&reactor_args.contract, &reactor_args.contract_version)
    else {
        return quote! {
            compile_error!("deployment descriptor requires contract and contract_version metadata");
        };
    };

    let contract_text = contract.value();
    if contract_text.is_empty()
        || contract_text.trim() != contract_text
        || contract_text.chars().any(char::is_control)
    {
        return syn::Error::new(
            contract.span(),
            "contract must be non-empty, contain no control characters, and have no surrounding whitespace",
        )
        .to_compile_error();
    }
    let contract_version = match contract_version.base10_parse::<u64>() {
        Ok(contract_version) => contract_version,
        Err(_) => {
            return syn::Error::new(contract_version.span(), "contract_version must fit in u64")
                .to_compile_error()
        }
    };

    if let Err(error) = model.body.validate_unique_reaction_names() {
        return error.to_compile_error();
    }

    if let Some(span) = model.body.unsupported_span() {
        let diagnostic = quote_spanned! {span =>
            compile_error!("deployment descriptor requires reaction! syntax");
        };
        return quote! {
            #diagnostic
        };
    }

    let name = &model.name;
    let reactor_name = ident_text(name);
    let reactor_path = stable_path_tokens(std::slice::from_ref(&reactor_name));
    let reactor_slot = quote! {
        ::boomerang::builder::ReactorSlot {
            id: ::boomerang::builder::ReactorSlotId::from_path(#reactor_path),
            parent: None,
        }
    };
    let port_names = model
        .args
        .iter()
        .filter_map(|arg| match arg.kind {
            ArgKind::Input { .. } | ArgKind::Output { .. } => Some(ident_text(&arg.name.ident)),
            ArgKind::State { .. } | ArgKind::Param { .. } => None,
        })
        .collect::<Vec<_>>();
    let port_slots = model.args.iter().filter_map(|arg| {
        let direction = match arg.kind {
            ArgKind::Input { .. } => quote!(::boomerang::builder::PortDirection::Input),
            ArgKind::Output { .. } => {
                quote!(::boomerang::builder::PortDirection::Output)
            }
            ArgKind::State { .. } | ArgKind::Param { .. } => return None,
        };
        let slot = stable_path_tokens(&[reactor_name.clone(), ident_text(&arg.name.ident)]);
        let reactor_path = stable_path_tokens(std::slice::from_ref(&reactor_name));
        Some(quote! {
            ::boomerang::builder::PortSlot {
                id: ::boomerang::builder::PortSlotId::from_path(#slot),
                reactor: ::boomerang::builder::ReactorSlotId::from_path(#reactor_path),
                direction: #direction,
            }
        })
    });
    let state_slots = model.args.iter().filter_map(|arg| {
        if !matches!(arg.kind, ArgKind::State { .. }) {
            return None;
        }
        let slot = stable_path_tokens(&[reactor_name.clone(), ident_text(&arg.name.ident)]);
        let reactor_path = stable_path_tokens(std::slice::from_ref(&reactor_name));
        Some(quote! {
            ::boomerang::builder::StateSlot {
                id: ::boomerang::builder::StateSlotId::from_path(#slot),
                reactor: ::boomerang::builder::ReactorSlotId::from_path(#reactor_path),
            }
        })
    });

    let modes = model
        .body
        .items
        .iter()
        .filter_map(|item| match item {
            BodyItem::Mode(mode) => Some(mode),
            BodyItem::Tokens(_) | BodyItem::Reaction(_) => None,
        })
        .collect::<Vec<_>>();
    let mode_names = modes
        .iter()
        .map(|mode| ident_text(&mode.name))
        .collect::<Vec<_>>();
    let mode_slots = modes.iter().map(|mode| {
        let slot = stable_path_tokens(&[reactor_name.clone(), ident_text(&mode.name)]);
        let reactor_path = stable_path_tokens(std::slice::from_ref(&reactor_name));
        let initial = mode.initial;
        quote! {
            ::boomerang::builder::ModeSlot {
                id: ::boomerang::builder::ModeSlotId::from_path(#slot),
                reactor: ::boomerang::builder::ReactorSlotId::from_path(#reactor_path),
                parent: None,
                initial: #initial,
            }
        }
    });

    let mut reactions = Vec::new();
    model.body.reactions(None, &mut reactions);
    let mut reaction_slots = Vec::new();
    let mut relationships = Vec::new();
    for (ordinal, (scope, reaction)) in reactions.into_iter().enumerate() {
        let reactor_path = stable_path_tokens(std::slice::from_ref(&reactor_name));
        let reaction_slot = if let Some(name) = reaction.name() {
            let path = stable_path_tokens(&[reactor_name.clone(), ident_text(name)]);
            quote!(::boomerang::builder::ReactionSlotId::from_path(#path))
        } else {
            let ordinal =
                u32::try_from(ordinal).expect("reaction count exceeds descriptor representation");
            quote!(::boomerang::builder::ReactionSlotId::from_path(
                (#reactor_path).append_generated_ordinal(#ordinal)
            ))
        };
        let owner_path = stable_path_tokens(std::slice::from_ref(&reactor_name));
        reaction_slots.push(quote! {
            ::boomerang::builder::ReactionSlot {
                id: #reaction_slot,
                reactor: ::boomerang::builder::ReactorSlotId::from_path(#owner_path),
            }
        });

        for (position, trigger) in reaction.triggers().iter().enumerate() {
            let target = match trigger {
                crate::reaction::TriggerType::Startup => quote! {
                    ::boomerang::builder::DescriptorRelationshipTarget::Lifecycle(
                        ::boomerang::builder::DescriptorLifecycle::Startup,
                    )
                },
                crate::reaction::TriggerType::Shutdown => quote! {
                    ::boomerang::builder::DescriptorRelationshipTarget::Lifecycle(
                        ::boomerang::builder::DescriptorLifecycle::Shutdown,
                    )
                },
                crate::reaction::TriggerType::Reset => quote! {
                    ::boomerang::builder::DescriptorRelationshipTarget::Lifecycle(
                        ::boomerang::builder::DescriptorLifecycle::Reset,
                    )
                },
                crate::reaction::TriggerType::Regular(path) => {
                    relationship_target(&reactor_name, &port_names, &mode_names, path).0
                }
            };
            relationships.push(relationship_tokens(
                &reaction_slot,
                quote!(::boomerang::builder::DescriptorRelationshipKind::Trigger),
                target,
                quote!(None),
                position,
            ));
        }
        for (position, path) in reaction.uses().iter().enumerate() {
            let target = relationship_target(&reactor_name, &port_names, &mode_names, path).0;
            relationships.push(relationship_tokens(
                &reaction_slot,
                quote!(::boomerang::builder::DescriptorRelationshipKind::Use),
                target,
                quote!(None),
                position,
            ));
        }
        for (position, effect) in reaction.effects().iter().enumerate() {
            let path = match effect {
                crate::reaction::EffectType::Regular(path)
                | crate::reaction::EffectType::Reset(path)
                | crate::reaction::EffectType::History(path) => path,
            };
            let (target, is_mode) =
                relationship_target(&reactor_name, &port_names, &mode_names, path);
            let is_transition = matches!(
                effect,
                crate::reaction::EffectType::Reset(_) | crate::reaction::EffectType::History(_)
            );
            let kind = if is_mode || is_transition {
                quote!(::boomerang::builder::DescriptorRelationshipKind::Mode)
            } else {
                quote!(::boomerang::builder::DescriptorRelationshipKind::Effect)
            };
            let transition = match effect {
                crate::reaction::EffectType::History(_) => quote! {
                    Some(::boomerang::builder::ModeTransitionKind::History)
                },
                crate::reaction::EffectType::Reset(_) => quote! {
                    Some(::boomerang::builder::ModeTransitionKind::Reset)
                },
                crate::reaction::EffectType::Regular(_) if is_mode => quote! {
                    Some(::boomerang::builder::ModeTransitionKind::Reset)
                },
                crate::reaction::EffectType::Regular(_) => quote!(None),
            };
            relationships.push(relationship_tokens(
                &reaction_slot,
                kind,
                target,
                transition,
                position,
            ));
        }
        if let Some(scope) = scope {
            let mode_slot = stable_path_tokens(&[reactor_name.clone(), ident_text(&scope.name)]);
            relationships.push(relationship_tokens(
                &reaction_slot,
                quote!(::boomerang::builder::DescriptorRelationshipKind::Scope),
                quote! {
                    ::boomerang::builder::DescriptorRelationshipTarget::Mode(
                        ::boomerang::builder::ModeSlotId::from_path(#mode_slot),
                    )
                },
                quote!(None),
                0,
            ));
        }
    }

    quote! {
        #[doc(hidden)]
        const ONLY_ONE_DEPLOYMENT_REACTOR_PER_MODULE: () = ();

        pub mod __boomerang {
            /// Returns the canonical host-owned component descriptor.
            pub fn descriptor() -> ::boomerang::builder::ComponentDescriptor {
                ::boomerang::builder::ComponentDescriptor::__from_macro(
                    #contract,
                    #contract_version,
                    ::boomerang::builder::COMPONENT_DESCRIPTOR_MACRO_ABI,
                    vec![#reactor_slot],
                    vec![#(#port_slots),*],
                    vec![],
                    vec![#(#reaction_slots),*],
                    vec![#(#mode_slots),*],
                    vec![#(#state_slots),*],
                    vec![],
                    vec![#(#relationships),*],
                    vec![],
                    vec![],
                    ::boomerang::builder::DescriptorBounds::default(),
                )
            }
        }
    }
}

fn relationship_tokens(
    reaction_slot: &TokenStream,
    kind: TokenStream,
    target: TokenStream,
    mode_transition: TokenStream,
    declaration_position: usize,
) -> TokenStream {
    let declaration_position = u32::try_from(declaration_position)
        .expect("reaction relationship count exceeds descriptor representation");
    quote! {
        ::boomerang::builder::DescriptorRelationship {
            reaction: #reaction_slot,
            kind: #kind,
            target: #target,
            mode_transition: #mode_transition,
            declaration_position: #declaration_position,
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
        if let Err(error) = self.0.validate() {
            tokens.append_all(error.to_compile_error());
            return;
        }
        let descriptor_output = descriptor_output(&self.0, &self.1);
        let conflict_output = quote! {
            compile_error!("__boomerang_descriptor and __boomerang_payload cannot both be enabled");
        };
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
        let payload_output = payload_output(
            reactor_args,
            &self.1,
            &state_type_path,
            &state_struct,
            &state_impl,
        );

        let port_idents = args
            .iter()
            .filter_map(|Arg { kind, name, .. }| match *kind {
                ArgKind::Input { .. } | ArgKind::Output { .. } => Some(name),
                _ => None,
            });

        //TODO for now param args are just re-built into the output function signature. In the future, I may want to generate a ctx type instead to support defaults.
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
                    let name_str = ident_text(&name.ident);
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
                                    let #name = ctx.add_input_ports::<#element_type, #len_expr>(#name_str)?;
                                }),
                                ArgKind::Output { .. } => Some(quote! {
                                    let #name = ctx.add_output_ports::<#element_type, #len_expr>(#name_str)?;
                                }),
                                _ => None,
                            }
                        }
                        _ => match (kind, len) {
                            (ArgKind::Input { .. }, Some(_)) => {
                                let len_name = format_ident!("{}_len", name.ident);
                                Some(quote! {
                                    let #name = ctx.add_input_bank::<#ty>(#name_str, #len_name)?;
                                    let #for_fn_name = #name.clone();
                                })
                            }
                            (ArgKind::Output { .. }, Some(_)) => {
                                let len_name = format_ident!("{}_len", name.ident);
                                Some(quote! {
                                    let #name = ctx.add_output_bank::<#ty>(#name_str, #len_name)?;
                                    let #for_fn_name = #name.clone();
                                })
                            }
                            _ => Some(quote! {
                                let #name = ctx.add_port::<#ty, #dir>(#name_str, None)?;
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
                         parent: Option<::boomerang::builder::AssemblyReactorKey>,
                         scope_mode: Option<::boomerang::builder::AssemblyModeKey>,
                         bank_info: Option<::boomerang::runtime::BankInfo>,
                         placement: ::boomerang::builder::ReactorPlacement,
                         assembly: &mut ::boomerang::builder::Assembly| {
                        #(#len_bindings)*
                        let mut ctx = assembly.add_reactor(name, parent, bank_info, state, placement);
                        if let Some(scope_mode) = scope_mode {
                            ctx.set_scope_mode(scope_mode)?;
                        }
                        #(#create_ports)*
                        (move |ctx: &mut ::boomerang::builder::ReactorContext<'_, #state_type_path>,
                              ports: (#(#local_types,)* )| -> Result<(), ::boomerang::builder::AssemblyError> {
                            #[allow(non_snake_case)]
                            let (#(#local_patterns,)*) = ports;
                            #(#mode_bindings)*
                            #body_tokens
                            Ok(())
                        })(&mut ctx, (#(#local_values,)*))?;
                        ctx.finish()?;
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
                        move |ctx, (#(#port_idents,)*)| {
                            #(#mode_bindings)*
                            #body_tokens
                            Ok(())
                        })
                }
            }
        };

        let facet_module = format_ident!("__boomerang_facets_for_{name}");
        tokens.append_all(quote! {
            #[allow(non_snake_case, unexpected_cfgs)]
            mod #facet_module {
                #[cfg(not(any(feature = "__boomerang_descriptor", feature = "__boomerang_payload")))]
                macro_rules! hosted { ($($tokens:tt)*) => { $($tokens)* }; }
                #[cfg(any(feature = "__boomerang_descriptor", feature = "__boomerang_payload"))]
                macro_rules! hosted { ($($tokens:tt)*) => {} }
                pub(super) use hosted;

                #[cfg(all(feature = "__boomerang_descriptor", not(feature = "__boomerang_payload")))]
                macro_rules! descriptor { ($($tokens:tt)*) => { $($tokens)* }; }
                #[cfg(not(all(feature = "__boomerang_descriptor", not(feature = "__boomerang_payload"))))]
                macro_rules! descriptor { ($($tokens:tt)*) => {} }
                pub(super) use descriptor;

                #[cfg(all(feature = "__boomerang_descriptor", feature = "__boomerang_payload"))]
                macro_rules! conflict { ($($tokens:tt)*) => { $($tokens)* }; }
                #[cfg(not(all(feature = "__boomerang_descriptor", feature = "__boomerang_payload")))]
                macro_rules! conflict { ($($tokens:tt)*) => {} }
                pub(super) use conflict;

                #[cfg(all(feature = "__boomerang_payload", not(feature = "__boomerang_descriptor")))]
                macro_rules! payload { ($($tokens:tt)*) => { $($tokens)* }; }
                #[cfg(not(all(feature = "__boomerang_payload", not(feature = "__boomerang_descriptor"))))]
                macro_rules! payload { ($($tokens:tt)*) => {} }
                pub(super) use payload;
            }

            #facet_module::hosted! { #output }
            #facet_module::descriptor! { #descriptor_output }
            #facet_module::payload! { #payload_output }
            #facet_module::conflict! { #conflict_output }
        });
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
