//! Emit host compiler declarations directly from the macro model.
use super::*;
use crate::reaction::{EffectType, PathOrIdent, TriggerType};

pub(super) fn output(args: &ReactorArgs, model: &Model) -> TokenStream {
    match generate(args, model) {
        Ok(tokens) => tokens,
        Err(error) => {
            // Preserve hosted compatibility. The unsupported definition deliberately does not
            // implement ComponentDefinition, so failure occurs when used for topology authoring.
            let reason = format!("topology constructor unavailable: {error}");
            let unavailable = format_ident!("{}TopologyUnavailable", model.name);
            quote! {
                #[doc = #reason]
                pub struct #unavailable;
                #[doc = #reason]
                #[deprecated(note = #reason)]
                pub fn definition() -> #unavailable { #unavailable }
            }
        }
    }
}

fn generate(args: &ReactorArgs, model: &Model) -> syn::Result<TokenStream> {
    let (Some(contract), Some(version)) = (&args.contract, &args.contract_version) else {
        return Err(syn::Error::new(
            model.name.span(),
            "requires contract and contract_version metadata",
        ));
    };
    let version = version.base10_parse::<u64>()?;
    if !model.generics.params.is_empty() || model.generics.where_clause.is_some() {
        return Err(syn::Error::new_spanned(
            &model.generics,
            "generic component declarations are not supported",
        ));
    }
    if let Some(span) = model.body.unsupported_span() {
        return Err(syn::Error::new(span, "requires declarative reaction! and mode! bodies; arbitrary builder statements are not supported"));
    }
    model.body.validate_unique_reaction_names()?;
    model.body.validate_unique_mode_names()?;
    let compiler = quote!(::boomerang::builder::compiler);
    let mut fields = Vec::new();
    let mut ports = Vec::new();
    let mut declarations = Vec::new();
    let mut targets = std::collections::BTreeMap::new();
    let mut action_position = 2u32;
    let mut port_position = 0u32;
    targets.insert("__startup".to_owned(), true);
    targets.insert("__shutdown".to_owned(), true);
    for arg in &model.args {
        let ident = &arg.name.ident;
        let name = ident_text(ident);
        let ty = &arg.ty;
        match &arg.kind {
            ArgKind::Input { len } | ArgKind::Output { len } => {
                if len.is_some() || matches!(ty, Type::Array(_)) {
                    return Err(syn::Error::new_spanned(
                        ty,
                        "port banks are not supported by topology constructors",
                    ));
                }
                let (handle, method) = if matches!(arg.kind, ArgKind::Input { .. }) {
                    (quote!(#compiler::TopologyInput), quote!(input))
                } else {
                    (quote!(#compiler::TopologyOutput), quote!(output))
                };
                let binding = format_ident!("__boomerang_topology_port_{}", name);
                fields.push(quote!(pub #ident: #handle<#ty>));
                ports.push(quote!(#ident: #binding));
                declarations
                    .push(quote!(let #binding = topology.#method::<#ty>(#name, #port_position)?;));
                port_position += 1;
                targets.insert(name, false);
            }
            ArgKind::LogicalAction { min_delay_nanos }
            | ArgKind::PhysicalAction { min_delay_nanos } => {
                let delay = min_delay_nanos.map_or(
                    quote!(None),
                    |nanos| quote!(Some(::boomerang::runtime::Duration::nanoseconds(#nanos))),
                );
                let kind = if matches!(arg.kind, ArgKind::LogicalAction { .. }) {
                    quote!(#compiler::ActionKind::Logical { minimum_delay: #delay })
                } else {
                    quote!(#compiler::ActionKind::Physical { minimum_delay: #delay })
                };
                declarations.push(quote! {
                    topology.builder().add_action(#compiler::ActionId::from_path(root.path().append_name(#name)?), root.clone(), #kind, #action_position, None)?;
                });
                action_position += 1;
                targets.insert(name, true);
            }
            ArgKind::State { .. } | ArgKind::Param { .. } => {}
        }
    }
    let mut mode_names = BTreeSet::new();
    for mode in model.body.modes() {
        let name = mode.name_str();
        let initial = mode.initial;
        mode_names.insert(name.clone());
        declarations.push(quote! {
            topology.builder().add_mode(#compiler::ModeId::from_path(root.path().append_name(#name)?), root.clone(), None, #initial)?;
        });
    }
    let mut reactions = Vec::new();
    model.body.reactions(None, &mut reactions);
    let mut next_generated = 0;
    for (scope, reaction) in reactions {
        let id = match CanonicalReactionSlot::for_reaction(reaction, &mut next_generated) {
            CanonicalReactionSlot::Named(name) => quote!(root.path().append_name(#name)?),
            CanonicalReactionSlot::Generated(ordinal) => {
                quote!(root.path().append_generated_ordinal(#ordinal))
            }
        };
        // Hosted ABI positions are independent action/port first-encounter sequences.
        let mut actions: Vec<(String, TokenStream)> = Vec::new();
        let mut ports_relations: Vec<(String, TokenStream)> = Vec::new();
        let mut record = |name: String, flags: TokenStream| -> syn::Result<()> {
            let Some(action) = targets.get(&name) else {
                return Err(syn::Error::new_spanned(
                    reaction.code(),
                    format!(
                        "topology relationship `{name}` must name a declared scalar port or action"
                    ),
                ));
            };
            let list = if *action {
                &mut actions
            } else {
                &mut ports_relations
            };
            if let Some((_, previous)) = list.iter_mut().find(|(target, _)| target == &name) {
                // Legacy declaration overwrites relation mode while retaining its first position.
                *previous = flags;
            } else {
                list.push((name, flags));
            }
            Ok(())
        };
        let mut reset = false;
        for trigger in reaction.triggers() {
            let name = match trigger {
                TriggerType::Startup => "__startup".to_owned(),
                TriggerType::Shutdown => "__shutdown".to_owned(),
                TriggerType::Reset => {
                    reset = true;
                    continue;
                }
                TriggerType::Regular(path) => scalar_name(path)?,
            };
            record(
                name,
                quote!(#compiler::ReactionRelationFlags::TRIGGER | #compiler::ReactionRelationFlags::USE),
            )?;
        }
        for path in reaction.uses() {
            record(
                scalar_name(path)?,
                quote!(#compiler::ReactionRelationFlags::USE),
            )?;
        }
        let mut transition = None;
        for effect in reaction.effects() {
            let path = match effect {
                EffectType::Regular(path) | EffectType::Reset(path) | EffectType::History(path) => {
                    path
                }
            };
            let name = scalar_name(path)?;
            if mode_names.contains(&name) {
                let history = matches!(effect, EffectType::History(_));
                if transition
                    .as_ref()
                    .is_some_and(|previous| previous != &(name.clone(), history))
                {
                    return Err(syn::Error::new_spanned(
                        path,
                        "topology reaction has inconsistent mode transitions",
                    ));
                }
                transition = Some((name, history));
            } else {
                if !matches!(effect, EffectType::Regular(_)) {
                    return Err(syn::Error::new_spanned(
                        path,
                        "reset/history effect requires a declared mode",
                    ));
                }
                record(name, quote!(#compiler::ReactionRelationFlags::EFFECT))?;
            }
        }
        let relations = actions.iter().enumerate().map(|(position, (name, flags))| {
            let position = position as u32;
            quote!(#compiler::ReactionRelation::new(#compiler::ReactionRelationTarget::Action(#compiler::ActionId::from_path(root.path().append_name(#name)?)), #flags, #position))
        }).chain(ports_relations.iter().enumerate().map(|(position, (name, flags))| {
            let position = position as u32;
            quote!(#compiler::ReactionRelation::new(#compiler::ReactionRelationTarget::Port(#compiler::PortId::from_path(root.path().append_name(#name)?)), #flags, #position))
        })).collect::<Vec<_>>();
        let mode = scope.map_or(quote!(None), |mode| {
            let name = mode.name_str();
            quote!(Some(#compiler::ModeId::from_path(root.path().append_name(#name)?)))
        });
        let reset_modes = if reset {
            let scope = scope.ok_or_else(|| {
                syn::Error::new_spanned(reaction.code(), "reset trigger requires a mode scope")
            })?;
            let name = scope.name_str();
            quote!(vec![#compiler::ModeId::from_path(root.path().append_name(#name)?)])
        } else {
            quote!(vec![])
        };
        let transition = transition.map_or(quote!(None), |(name, history)| {
            let kind = if history { quote!(#compiler::ModeTransitionKind::History) } else { quote!(#compiler::ModeTransitionKind::Reset) };
            quote!(Some(#compiler::ModeTransition::new(#compiler::ModeId::from_path(root.path().append_name(#name)?), #kind)))
        });
        declarations.push(quote! {
            topology.builder().add_reaction(#compiler::ReactionId::from_path(#id), root.clone(), [#(#relations),*], #compiler::ReactionOptions { mode: #mode, enabled_modes: (#mode).into_iter().collect(), reset_modes: #reset_modes, transition: #transition })?;
        });
    }
    let definition_name = format_ident!("{}Topology", model.name);
    let ports_name = format_ident!("{}TopologyPorts", model.name);
    Ok(quote! {
        /// Typed public topology ports; constructing them does not initialize reactor state.
        pub struct #ports_name { #(#fields,)* }
        /// Target-neutral declarations generated from this component's reactor model.
        pub struct #definition_name;
        /// Describe this component without constructing runtime state.
        pub fn definition() -> #definition_name { #definition_name }
        impl #compiler::ComponentDefinition for #definition_name {
            type Ports = #ports_name;
            fn contract(&self) -> (&str, u64) { (#contract, #version) }
            fn declare(&self, topology: &mut #compiler::ComponentTopology<'_>) -> Result<Self::Ports, #compiler::TopologyAuthoringError> {
                let root = topology.root().clone();
                #(#declarations)*
                Ok(#ports_name { #(#ports,)* })
            }
        }
    })
}

fn scalar_name(path: &PathOrIdent) -> syn::Result<String> {
    match path {
        PathOrIdent::Simple(ident) => Ok(ident_text(ident)),
        PathOrIdent::Field(_) => Err(syn::Error::new_spanned(path, "topology constructors require scalar local relationships; child reactor paths are not supported")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_declarations_explain_constructor_limits() {
        let args: ReactorArgs =
            syn::parse_str("contract = \"test\", contract_version = 1").unwrap();
        for (source, expected) in [
            (
                "fn Root<T>(#[input] value: T) {}",
                "generic component declarations",
            ),
            ("fn Root(#[input] values: [u32; 2]) {}", "port banks"),
            (
                "fn Root() { let action = ctx.add_logical_action::<()>(\"a\", None)?; }",
                "arbitrary builder statements",
            ),
        ] {
            let model: Model = syn::parse_str(source).unwrap();
            assert!(generate(&args, &model)
                .unwrap_err()
                .to_string()
                .contains(expected));
        }
    }
}
