use proc_macro2::{Delimiter, Span, TokenStream, TokenTree};
use proc_macro_error2::abort;
use quote::{format_ident, quote, quote_spanned, ToTokens, TokenStreamExt};
use std::collections::BTreeSet;
use syn::{
    braced, ext::IdentExt, parse::Parse, parse_quote, spanned::Spanned, Attribute, FnArg, Ident,
    Meta, Pat, PatIdent, Signature, Type, TypePath, Visibility,
};

use crate::util::convert_from_snake_case;

pub(crate) mod emission;
mod init_context;
mod semantic;
mod topology;

pub(crate) use emission::ComponentEmission;

/// Top-level arguments for the `#[reactor]` macro.
#[derive(attribute_derive::FromAttr)]
pub struct ReactorArgs {
    /// The name of the state type to use. If not provided, a state struct will be generated.
    state: Option<TypePath>,
    /// State constructor. Receives the generated context when `init_context` is set.
    state_init: Option<syn::Path>,
    /// Fields of the generated context passed to the state constructor.
    init_context: Option<init_context::InitContextArgs>,
    /// Stable component contract identity for descriptor builds.
    contract: Option<syn::LitStr>,
    /// Stable component contract version for descriptor builds.
    contract_version: Option<syn::LitInt>,
    /// Finite component resource bounds required for static lowering.
    bounds: Option<DescriptorBoundsArgs>,
}

/// Parsed values from the nested `bounds(...)` reactor argument.
#[derive(attribute_derive::FromAttr)]
struct DescriptorBoundsArgs {
    /// Maximum queued events.
    queue_capacity: syn::LitInt,
    /// Maximum payload storage in bytes.
    payload_bytes: syn::LitInt,
    /// Maximum reactor-state storage in bytes.
    state_bytes: syn::LitInt,
    /// Maximum scheduler scratch storage in bytes.
    scratch_bytes: syn::LitInt,
}

/// Validated numeric descriptor bounds from the declaration.
struct DescriptorBoundsValues {
    queue_capacity: u64,
    payload_bytes: u64,
    state_bytes: u64,
    scratch_bytes: u64,
}

impl ReactorArgs {
    fn validate(&self) -> syn::Result<()> {
        if self.init_context.is_some() && (self.state.is_none() || self.state_init.is_none()) {
            return Err(syn::Error::new(
                Span::call_site(),
                "init_context requires state and state_init",
            ));
        }
        if let (Some(state_init), None) = (&self.state_init, &self.state) {
            return Err(syn::Error::new_spanned(
                state_init,
                "`state_init` requires `state = T`",
            ));
        }
        if self.contract.is_some() != self.contract_version.is_some() {
            return Err(syn::Error::new(
                Span::call_site(),
                "deployment descriptor requires contract and contract_version metadata",
            ));
        }
        if self.contract.is_some() != self.bounds.is_some() {
            return Err(syn::Error::new(
                Span::call_site(),
                "deployment descriptor requires bounds metadata with contract metadata",
            ));
        }
        Ok(())
    }

    /// Converts declared bounds into the shared descriptor representation.
    fn descriptor_bounds(&self) -> syn::Result<DescriptorBoundsValues> {
        let Some(bounds) = &self.bounds else {
            return Ok(DescriptorBoundsValues {
                queue_capacity: u64::MAX,
                payload_bytes: u64::MAX,
                state_bytes: u64::MAX,
                scratch_bytes: u64::MAX,
            });
        };
        let known = |value: &syn::LitInt| {
            value
                .base10_parse::<u64>()
                .map_err(|_| syn::Error::new(value.span(), "resource bound must fit in u64"))
        };
        Ok(DescriptorBoundsValues {
            queue_capacity: known(&bounds.queue_capacity)?,
            payload_bytes: known(&bounds.payload_bytes)?,
            state_bytes: known(&bounds.state_bytes)?,
            scratch_bytes: known(&bounds.scratch_bytes)?,
        })
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
    LogicalAction { min_delay_nanos: Option<i64> },
    PhysicalAction { min_delay_nanos: Option<i64> },
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
            } else if attr.path().is_ident("logical_action") {
                if kind.is_some() {
                    abort!(attr, "duplicate argument kind");
                }
                kind = Some(ArgKind::LogicalAction {
                    min_delay_nanos: parse_action_min_delay(attr)?,
                });
            } else if attr.path().is_ident("physical_action") {
                if kind.is_some() {
                    abort!(attr, "duplicate argument kind");
                }
                kind = Some(ArgKind::PhysicalAction {
                    min_delay_nanos: parse_action_min_delay(attr)?,
                });
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
                    "unknown attribute, expected one of: input, output, logical_action, physical_action, state, param"
                );
            }
        }

        Ok(match kind {
            Some(k) => k,
            None => ArgKind::Param { default: None },
        })
    }
}

fn parse_action_min_delay(attr: &Attribute) -> syn::Result<Option<i64>> {
    let Meta::List(list) = &attr.meta else {
        return match &attr.meta {
            Meta::Path(_) => Ok(None),
            _ => Err(syn::Error::new_spanned(
                attr,
                "expected `min_delay = <duration>` in action attribute",
            )),
        };
    };
    if list.tokens.is_empty() {
        return Ok(None);
    }

    list.parse_args_with(|input: syn::parse::ParseStream<'_>| {
        let key: Ident = input.parse()?;
        if key != "min_delay" {
            return Err(syn::Error::new(
                key.span(),
                "expected `min_delay = <duration>` in action attribute",
            ));
        }
        input.parse::<syn::Token![=]>()?;
        let duration = input.parse::<crate::time::Dur>().map_err(|error| {
            syn::Error::new(
                error.span(),
                format!("invalid action minimum delay: {error}"),
            )
        })?;
        if !input.is_empty() {
            return Err(input.error("unexpected trailing content in action attribute"));
        }
        let nanos = duration.0.as_nanos();
        i64::try_from(nanos).map(Some).map_err(|_| {
            syn::Error::new(
                list.tokens.span(),
                "action minimum delay exceeds the runtime nanosecond range",
            )
        })
    })
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
        self.body_tokens_with_descriptor_slots(None, &mut 0)
    }

    /// Emits dynamic body tokens while assigning descriptor reaction slots in source order.
    fn body_tokens_with_descriptor_slots(
        &self,
        reactor_name: Option<&str>,
        ordinal: &mut u32,
    ) -> TokenStream {
        let mut tokens = TokenStream::new();
        for item in &self.items {
            match item {
                BodyItem::Tokens(body_tokens) => tokens.append_all(body_tokens.clone()),
                BodyItem::Reaction(reaction) => {
                    let descriptor_slot =
                        reactor_name.map(|reactor_name| match CanonicalReactionSlot::for_reaction(
                            reaction, ordinal,
                        ) {
                            CanonicalReactionSlot::Named(name) => {
                                let path = stable_path_tokens(&[reactor_name.to_owned(), name]);
                                quote!(::boomerang::builder::ReactionSlotId::from_path(#path))
                            }
                            CanonicalReactionSlot::Generated(ordinal) => {
                                let reactor_path = stable_path_tokens(&[reactor_name.to_owned()]);
                                quote!(::boomerang::builder::ReactionSlotId::from_path(
                                    (#reactor_path).append_generated_ordinal(#ordinal)
                                ))
                            }
                        });
                    reaction.to_tokens_with_descriptor_slot(&mut tokens, descriptor_slot);
                }
                BodyItem::Mode(mode) => {
                    let key_ident = mode.key_ident();
                    let body = mode
                        .body
                        .body_tokens_with_descriptor_slots(reactor_name, ordinal);
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

    fn validate_unique_mode_names(&self) -> syn::Result<()> {
        let mut names = BTreeSet::new();
        for mode in self.modes() {
            if !names.insert(ident_text(&mode.name)) {
                return Err(syn::Error::new(
                    mode.name.span(),
                    format!("duplicate mode name `{}`", mode.name),
                ));
            }
        }
        Ok(())
    }

    fn modes(&self) -> impl Iterator<Item = &ModeDecl> {
        self.items.iter().filter_map(|item| match item {
            BodyItem::Mode(mode) => Some(mode),
            BodyItem::Tokens(_) | BodyItem::Reaction(_) => None,
        })
    }
}

/// Canonical descriptor-local identity segment for one macro reaction.
enum CanonicalReactionSlot {
    /// User-declared reaction name.
    Named(String),
    /// Zero-based ordinal among anonymous reactions only.
    Generated(u32),
}

impl CanonicalReactionSlot {
    /// Resolves a named or generated slot while advancing only anonymous ordinals.
    fn for_reaction(reaction: &crate::reaction::Model, next_generated: &mut u32) -> Self {
        if let Some(name) = reaction.name() {
            Self::Named(ident_text(name))
        } else {
            let ordinal = *next_generated;
            *next_generated = next_generated
                .checked_add(1)
                .expect("anonymous reaction count exceeds descriptor representation");
            Self::Generated(ordinal)
        }
    }

    /// Formats the slot segment for generated binding symbols.
    fn text(self) -> String {
        match self {
            Self::Named(name) => name,
            Self::Generated(ordinal) => format!("#g{ordinal}"),
        }
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

fn required_binding_symbol(prefix: &str, segments: &[&str]) -> Ident {
    use std::fmt::Write as _;

    let mut symbol = String::from(prefix);
    for byte in segments.join("/").bytes() {
        if byte.is_ascii_alphanumeric() {
            symbol.push(char::from(byte));
        } else {
            write!(symbol, "_{byte:02x}").expect("writing into a String cannot fail");
        }
    }
    format_ident!("{symbol}")
}

/// Resolves one own-relation reference to its concrete payload parameter type.
fn payload_ref(
    reactor_name: &str,
    args: &[Arg],
    mode_names: &[String],
    path: &crate::reaction::PathOrIdent,
    store_lifetime: &syn::Lifetime,
) -> syn::Result<(Ident, TokenStream)> {
    let crate::reaction::PathOrIdent::Simple(ident) = path else {
        return Err(syn::Error::new_spanned(
            path,
            "payload mode supports only own ports, modes, and lifecycle relations",
        ));
    };
    let name = ident_text(ident);
    if mode_names.contains(&name) {
        return Ok((
            ident.clone(),
            quote!(::boomerang::runtime::CompiledModeEffectRef),
        ));
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
            quote!([::boomerang::runtime::InputRef<#store_lifetime, #element>; #len])
        }
        (ArgKind::Output { len: None }, Type::Array(array)) => {
            let element = &array.elem;
            let len = &array.len;
            quote!([::boomerang::runtime::OutputRef<#store_lifetime, #element>; #len])
        }
        (ArgKind::Input { len: Some(_) }, ty) => {
            quote!(::boomerang::runtime::InputBankRef<#store_lifetime, #ty>)
        }
        (ArgKind::Output { len: Some(_) }, ty) => {
            quote!(::boomerang::runtime::OutputBankRef<#store_lifetime, #ty>)
        }
        (ArgKind::Input { len: None }, ty) => {
            quote!(::boomerang::runtime::InputRef<#store_lifetime, #ty>)
        }
        (ArgKind::Output { len: None }, ty) => {
            quote!(::boomerang::runtime::OutputRef<#store_lifetime, #ty>)
        }
        (ArgKind::LogicalAction { .. } | ArgKind::PhysicalAction { .. }, ty) => {
            quote!(::boomerang::runtime::ActionRef<#store_lifetime, #ty>)
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

/// Chooses a generated payload-reference lifetime absent from user generics.
fn fresh_payload_lifetime(generics: &syn::Generics) -> syn::Lifetime {
    for ordinal in 0u32.. {
        let candidate = if ordinal == 0 {
            "__boomerang_store".to_owned()
        } else {
            format!("__boomerang_store_{ordinal}")
        };
        if generics
            .lifetimes()
            .all(|parameter| parameter.lifetime.ident != candidate)
        {
            return syn::Lifetime::new(&format!("'{candidate}"), Span::call_site());
        }
    }
    unreachable!("u32 payload lifetime namespace exhausted")
}

/// Emits typed payload functions for every descriptor-local reaction slot.
fn payload_reaction_output(
    model: &Model,
    state_type: &syn::Path,
    mode_names: &[String],
    symbols: &mut PayloadSymbols,
) -> syn::Result<Vec<TokenStream>> {
    let reactor_name = ident_text(&model.name);
    let store_lifetime = fresh_payload_lifetime(&model.generics);
    let mut reactions = Vec::new();
    model.body.reactions(None, &mut reactions);
    let mut next_generated = 0;
    reactions
        .into_iter()
        .map(|(_, reaction)| {
            let reaction_name =
                CanonicalReactionSlot::for_reaction(reaction, &mut next_generated).text();
            let symbol = required_binding_symbol("reaction_", &[&reactor_name, &reaction_name]);
            let symbol = symbols.definition(&symbol);
            let export_doc = format!(
                "Invokes payload reaction `{reactor_name}/{reaction_name}` with concrete references."
            );
            let mut refs = Vec::new();
            for trigger in reaction.triggers() {
                match trigger {
                    crate::reaction::TriggerType::Startup => refs.push((
                        format_ident!("startup"),
                        quote!(::boomerang::runtime::ActionRef<#store_lifetime>),
                    )),
                    crate::reaction::TriggerType::Shutdown => refs.push((
                        format_ident!("shutdown"),
                        quote!(::boomerang::runtime::ActionRef<#store_lifetime>),
                    )),
                    crate::reaction::TriggerType::Reset => {}
                    crate::reaction::TriggerType::Regular(path) => {
                        refs.push(payload_ref(
                            &reactor_name,
                            &model.args,
                            mode_names,
                            path,
                            &store_lifetime,
                        )?);
                    }
                }
            }
            for path in reaction.uses() {
                refs.push(payload_ref(
                    &reactor_name,
                    &model.args,
                    mode_names,
                    path,
                    &store_lifetime,
                )?);
            }
            for effect in reaction.effects() {
                let path = match effect {
                    crate::reaction::EffectType::Regular(path)
                    | crate::reaction::EffectType::Reset(path)
                    | crate::reaction::EffectType::History(path) => path,
                };
                refs.push(payload_ref(
                    &reactor_name,
                    &model.args,
                    mode_names,
                    path,
                    &store_lifetime,
                )?);
            }
            let (ref_names, ref_types): (Vec<_>, Vec<_>) = refs.into_iter().unzip();
            let code = reaction.code();
            let mut generics = model.generics.clone();
            generics.params.insert(
                0,
                syn::GenericParam::Lifetime(syn::LifetimeParam::new(store_lifetime.clone())),
            );
            let (impl_generics, _, where_clause) = generics.split_for_impl();
            Ok(quote! {
                #[doc = #export_doc]
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

/// Native definitions stay in the declaration's lexical scope. Stable public binding names are
/// reexported from the nested namespace, preserving `self::`/`super::` paths without rewriting them.
struct PayloadSymbols {
    /// Public reexports that map implementation-local names to stable payload binding symbols.
    aliases: Vec<TokenStream>,
}
impl PayloadSymbols {
    fn definition(&mut self, public: &Ident) -> Ident {
        let implementation = format_ident!("__boomerang_payload_{public}");
        self.aliases.push(quote! {
            pub use super::super::#implementation as #public;
        });
        implementation
    }
}

/// Keep owning-module implementation declarations separate from the public binding namespace.
struct PayloadOutput {
    /// State types, capability contexts, and implementation definitions emitted in their owning scope.
    declarations: TokenStream,
    /// Compatibility manifest and binding symbols emitted in the public payload namespace.
    exports: TokenStream,
}
impl PayloadOutput {
    fn diagnostic(declarations: TokenStream) -> Self {
        Self {
            declarations,
            exports: TokenStream::new(),
        }
    }
}

fn payload_output(
    reactor_args: &ReactorArgs,
    model: &Model,
    state_type: &syn::Path,
    state_struct: &Option<TokenStream>,
    state_impl: &Option<TokenStream>,
    semantic: &syn::Result<Option<boomerang_builder::ComponentDescriptor>>,
) -> PayloadOutput {
    let descriptor = match semantic {
        Ok(Some(descriptor)) => descriptor,
        Ok(None) => {
            return PayloadOutput::diagnostic(
                quote!(compile_error!("compiled component requires contract and contract_version metadata");),
            )
        }
        Err(error) => return PayloadOutput::diagnostic(error.to_compile_error()),
    };
    if reactor_args.state.is_some() && reactor_args.state_init.is_none() {
        return PayloadOutput::diagnostic(
            quote! { compile_error!("payload mode requires `state_init = path` with `state = T`"); },
        );
    }

    let reactor_name = ident_text(&model.name);
    let mode_names = model
        .body
        .modes()
        .map(|mode| ident_text(&mode.name))
        .collect::<Vec<_>>();
    let mut symbols = PayloadSymbols {
        aliases: Vec::new(),
    };
    let reaction_exports =
        match payload_reaction_output(model, state_type, &mode_names, &mut symbols) {
            Ok(exports) => exports,
            Err(error) => return PayloadOutput::diagnostic(error.to_compile_error()),
        };
    let macro_abi = boomerang_runtime::binding::COMPONENT_DESCRIPTOR_MACRO_ABI;
    let fingerprint = descriptor.fingerprint().to_bytes();
    let binding_manifest = quote! {
        /// Actual compatibility information derived from this component declaration.
        pub const fn binding_manifest() -> ::boomerang::runtime::binding::BindingManifest {
            ::boomerang::runtime::binding::BindingManifest::new(
                ::boomerang::runtime::binding::DescriptorFingerprint::new([#(#fingerprint),*]),
                #macro_abi,
            )
        }
    };
    let binding_exports = model
        .args
        .iter()
        .filter_map(|arg| {
            let name = ident_text(&arg.name.ident);
            let (symbol, payload_type, kind) = match (&arg.kind, &arg.ty) {
                (ArgKind::Input { .. } | ArgKind::Output { .. }, Type::Array(array)) => (
                    required_binding_symbol("port_", &[&reactor_name, &name]),
                    array.elem.as_ref(),
                    "port",
                ),
                (ArgKind::Input { .. } | ArgKind::Output { .. }, ty) => (
                    required_binding_symbol("port_", &[&reactor_name, &name]),
                    ty,
                    "port",
                ),
                (ArgKind::LogicalAction { .. } | ArgKind::PhysicalAction { .. }, ty) => (
                    required_binding_symbol("action_", &[&reactor_name, &name]),
                    ty,
                    "action",
                ),
                (ArgKind::State { .. } | ArgKind::Param { .. }, _) => return None,
            };
            let symbol = symbols.definition(&symbol);
            let export_doc = format!("Identifies payload type for {kind} `{reactor_name}/{name}`.");
            Some(quote! {
                #[doc = #export_doc]
                #[allow(non_upper_case_globals)]
                pub const #symbol: ::boomerang::runtime::PayloadType<#payload_type> =
                    ::boomerang::runtime::PayloadType::new();
            })
        })
        .collect::<Vec<_>>();
    let state_symbol = required_binding_symbol("state_", &[&reactor_name]);
    let state_type_symbol = format_ident!("type_{state_symbol}");
    let state_symbol = symbols.definition(&state_symbol);
    let state_type_symbol = symbols.definition(&state_type_symbol);
    let mut state_witness_generics = model.generics.clone();
    state_witness_generics
        .make_where_clause()
        .predicates
        .push(syn::parse_quote!(#state_type: ::boomerang::runtime::ReactorData));
    let (state_type_generics, _, state_type_where) = state_witness_generics.split_for_impl();
    let capability_declaration = match init_context::declaration(reactor_args, model) {
        Ok(value) => value,
        Err(error) => return PayloadOutput::diagnostic(error.to_compile_error()),
    };
    let state_initializer = if reactor_args.init_context.is_some() {
        match init_context::initializer(reactor_args, model, &state_symbol, state_type) {
            Ok(value) => value,
            Err(error) => return PayloadOutput::diagnostic(error.to_compile_error()),
        }
    } else {
        let state_init = reactor_args.state_init.as_ref().map_or_else(
            || quote!(::core::default::Default::default()),
            |path| quote!(#path()),
        );
        let (state_generics, _, state_where_clause) = model.generics.split_for_impl();
        quote! {
            /// Constructs this reactor's concrete payload state.
            #[allow(non_snake_case)]
            pub fn #state_symbol #state_generics() -> #state_type #state_where_clause { #state_init }
        }
    };

    let state_type_export = quote! {
        /// Identifies concrete state independently of its initialization signature.
        #[allow(non_snake_case)]
        pub fn #state_type_symbol #state_type_generics() -> ::boomerang::runtime::PayloadType<#state_type> #state_type_where { ::boomerang::runtime::PayloadType::new() }
    };
    let implementations = std::iter::once(state_type_export)
        .chain(std::iter::once(state_initializer))
        .chain(binding_exports)
        .chain(reaction_exports)
        .map(|definition| quote!(#[doc(hidden)] #definition));
    let aliases = symbols.aliases;
    PayloadOutput {
        declarations: quote! {
            #state_struct
            #state_impl
            #capability_declaration
            #(#implementations)*
        },
        exports: quote! { #binding_manifest #(#aliases)* },
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

pub struct ArgsModel(pub ReactorArgs, pub Model, pub bool);

impl ToTokens for ArgsModel {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        self.to_tokens_with_emission(tokens, ComponentEmission::Dynamic);
    }
}

impl ArgsModel {
    pub(crate) fn to_tokens_with_emission(
        &self,
        tokens: &mut TokenStream,
        emission: ComponentEmission,
    ) {
        if let Err(error) = self.0.validate() {
            tokens.append_all(error.to_compile_error());
            return;
        }
        let selection = emission::SelectionPolicy::new(emission);
        let descriptor = semantic::parse_descriptor(&self.0, &self.1);
        let descriptor_contents = match &descriptor {
            Ok(Some(descriptor)) => semantic::emit_descriptor(descriptor),
            Ok(None) => TokenStream::new(),
            Err(error) => error.to_compile_error(),
        };
        let topology_output = if self.2 {
            topology::output(&self.0, &self.1)
        } else {
            TokenStream::new()
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
            _,
        ) = self;

        // Extract generics parts
        let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

        // name of the generated Ports struct
        let ports_name = format_ident!("{name}Ports");

        let reactor_name = ident_text(name);
        let state_args = args
            .iter()
            .filter_map(
                |Arg {
                     docs,
                     kind,
                     name,
                     ty,
                 }| match kind {
                    ArgKind::State { default: _ } => {
                        let field_doc = format!(
                            "State slot `{reactor_name}/{}` owned by this reactor instance.",
                            ident_text(&name.ident)
                        );
                        Some(quote! { #[doc = #field_doc] #docs pub #name: #ty })
                    }
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

        let (state_struct, payload_state_struct) = if reactor_args.state.is_none() {
            if state_args.is_empty() {
                (
                    Some(quote! {
                        #vis type #state_ident = ();
                    }),
                    Some(quote! {
                        pub type #state_ident = ();
                    }),
                )
            } else {
                (
                    Some(quote! {
                        #[derive(Clone)]
                        #vis struct #state_ident #impl_generics #where_clause {
                            #(#state_args),*
                        }
                    }),
                    Some(quote! {
                        #[derive(Clone)]
                        pub struct #state_ident #impl_generics #where_clause {
                            #(#state_args),*
                        }
                    }),
                )
            }
        } else {
            (None, None)
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
        let PayloadOutput {
            declarations: native_payload_output,
            exports: native_payload_exports,
        } = if selection.needs_payload() {
            self::payload_output(
                reactor_args,
                &self.1,
                &state_type_path,
                &payload_state_struct,
                &state_impl,
                &descriptor,
            )
        } else {
            PayloadOutput::diagnostic(TokenStream::new())
        };

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
        let action_bindings = args.iter().filter_map(|Arg { kind, name, ty, .. }| {
            let (method, min_delay_nanos) = match kind {
                ArgKind::LogicalAction { min_delay_nanos } => {
                    (format_ident!("add_logical_action"), min_delay_nanos)
                }
                ArgKind::PhysicalAction { min_delay_nanos } => {
                    (format_ident!("add_physical_action"), min_delay_nanos)
                }
                ArgKind::Input { .. }
                | ArgKind::Output { .. }
                | ArgKind::State { .. }
                | ArgKind::Param { .. } => return None,
            };
            let name_text = ident_text(&name.ident);
            let min_delay = min_delay_nanos.map_or_else(
                || quote!(None),
                |nanos| quote!(Some(::boomerang::runtime::Duration::nanoseconds(#nanos))),
            );
            Some(quote! {
                let #name = ctx.#method::<#ty>(#name_text, #min_delay)?;
            })
        });
        let (component_metadata, body_tokens) = match (
            reactor_args.contract.as_ref(),
            reactor_args.contract_version.as_ref(),
        ) {
            (Some(contract), Some(version)) => {
                let version = match version.base10_parse::<u64>() {
                    Ok(version) => version,
                    Err(_) => {
                        tokens.append_all(
                            syn::Error::new(version.span(), "contract_version must fit in u64")
                                .to_compile_error(),
                        );
                        return;
                    }
                };
                let reactor_name = ident_text(name);
                let body_tokens =
                    body.body_tokens_with_descriptor_slots(Some(&reactor_name), &mut 0);
                (
                    quote! {
                        ctx.set_component_contract(
                            ::boomerang::builder::compiler::ContractId::new(#contract)
                                .expect("reactor macro validated contract text"),
                            #version,
                        );
                    },
                    body_tokens,
                )
            }
            (None, None) => (TokenStream::new(), body.body_tokens()),
            _ => unreachable!("reactor argument validation requires complete contract metadata"),
        };

        let capability_mode = reactor_args.init_context.is_some();
        let live_registration = match init_context::live_registration(reactor_args, &self.1) {
            Ok(tokens) => tokens,
            Err(error) => {
                tokens.append_all(error.to_compile_error());
                return;
            }
        };
        let state_parameter = if capability_mode {
            quote!(Option<#state_type_path>)
        } else {
            quote!(#state_type_path)
        };
        let context_creation = if capability_mode {
            quote!(let mut ctx = ::boomerang::builder::ReactorContext::<#state_type_path>::new_optional(name, parent, bank_info, state, placement, assembly);)
        } else {
            quote!(let mut ctx = assembly.add_reactor(name, parent, bank_info, state, placement);)
        };
        let adapter = capability_mode.then(|| quote!(::boomerang::builder::CapabilityReactor));
        let output = if has_banked_ports || capability_mode {
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

                        let direction = if matches!(kind, ArgKind::Input { .. }) {
                            "input"
                        } else {
                            "output"
                        };
                        let field_doc = format!(
                            "Typed {direction} port handle for `{reactor_name}/{}` owned by this reactor instance.",
                            ident_text(&name.ident)
                        );
                        Some(quote! { #[doc = #field_doc] #docs pub #name: #field_ty })
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
                    #adapter (move |name: &str,
                         state: #state_parameter,
                         parent: Option<::boomerang::builder::AssemblyReactorKey>,
                         scope_mode: Option<::boomerang::builder::AssemblyModeKey>,
                         bank_info: Option<::boomerang::runtime::BankInfo>,
                         placement: ::boomerang::builder::ReactorPlacement,
                         assembly: &mut ::boomerang::builder::Assembly| {
                        #(#len_bindings)*
                        #context_creation
                        #component_metadata
                        if let Some(scope_mode) = scope_mode {
                            ctx.set_scope_mode(scope_mode)?;
                        }
                        #(#create_ports)*
                        #(#action_bindings)*
                        #live_registration
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
                    })
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
                            #component_metadata
                            #(#action_bindings)*
                            #(#mode_bindings)*
                            #body_tokens
                            Ok(())
                        })
                }
            }
        };

        let capability_declaration = match init_context::declaration(reactor_args, &self.1) {
            Ok(value) => value,
            Err(error) => {
                tokens.append_all(error.to_compile_error());
                return;
            }
        };
        let metadata_contents = if matches!(&descriptor, Ok(None)) {
            quote!(compile_error!("compiled component requires contract and contract_version metadata");)
        } else {
            descriptor_contents
        };
        tokens.append_all(selection.output(
            name,
            emission::ReactorOutput {
                dynamic: quote!(#capability_declaration #output #topology_output),
                native_metadata: metadata_contents,
                topology: topology_output,
                native_payload: native_payload_output,
                native_payload_exports,
            },
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_reactor_emits_an_unconditional_dynamic_constructor() {
        let model = syn::parse_str::<Model>("pub fn Dynamic() -> impl Reactor {}").unwrap();
        let args = syn::parse_str::<ReactorArgs>("").unwrap();
        let declaration = ArgsModel(args, model, false);
        let output = syn::parse2::<syn::File>(quote!(#declaration)).unwrap();
        assert!(
            output.items.iter().any(|item| matches!(item,
                syn::Item::Fn(function) if function.sig.ident == "Dynamic"
            )),
            "direct reactors must expose the dynamic constructor without a selector"
        );
    }

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
