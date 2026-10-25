//! Emit host compiler declarations directly from the macro model.
use super::*;
use crate::reaction::PathOrIdent;
use boomerang_builder::{
    DescriptorLifecycle, DescriptorRelationshipKind as Kind, ModeTransitionKind,
};

pub(super) fn output(
    args: &ReactorArgs,
    declaration: &declaration::Declaration<'_>,
) -> TokenStream {
    let model = declaration.model;
    match generate(args, declaration) {
        Ok(tokens) => tokens,
        Err(error) => {
            // Preserve dynamic compatibility. The unsupported definition deliberately does not
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

fn generate(
    args: &ReactorArgs,
    declaration: &declaration::Declaration<'_>,
) -> syn::Result<TokenStream> {
    let model = declaration.model;
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
    if let Some(span) = declaration.unsupported {
        return Err(syn::Error::new(span, "requires declarative reaction! and mode! bodies; arbitrary builder statements are not supported"));
    }
    declaration.validate_names()?;
    let compiler = quote!(::boomerang::builder::compiler);
    let mut fields = Vec::new();
    let mut ports = Vec::new();
    let mut declarations = Vec::new();
    let mut action_position = 2u32;
    let mut port_position = 0u32;
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
                let direction = if matches!(arg.kind, ArgKind::Input { .. }) {
                    "input"
                } else {
                    "output"
                };
                let field_doc = format!(
                    "Typed topology {direction} port `{name}` owned by this component instance."
                );
                fields.push(quote!(#[doc = #field_doc] pub #ident: #handle<#ty>));
                ports.push(quote!(#ident: #binding));
                declarations
                    .push(quote!(let #binding = topology.#method::<#ty>(#name, #port_position)?;));
                port_position += 1;
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
            }
            ArgKind::State { .. } | ArgKind::Param { .. } => {}
        }
    }
    for mode in &declaration.modes {
        let name = mode.name_str();
        let initial = mode.initial;
        declarations.push(quote! {
            topology.builder().add_mode(#compiler::ModeId::from_path(root.path().append_name(#name)?), root.clone(), None, #initial)?;
        });
    }
    for reaction in &declaration.reactions {
        let scope = reaction.scope;
        let id = match &reaction.slot {
            CanonicalReactionSlot::Named(name) => quote!(root.path().append_name(#name)?),
            CanonicalReactionSlot::Generated(ordinal) => {
                quote!(root.path().append_generated_ordinal(#ordinal))
            }
        };
        // Dynamic ABI positions are independent action/port first-encounter sequences.
        let mut actions: Vec<(String, TokenStream)> = Vec::new();
        let mut ports_relations: Vec<(String, TokenStream)> = Vec::new();
        let mut record = |name: String,
                          relation: &declaration::Relation<'_>,
                          flags: TokenStream|
         -> syn::Result<()> {
            let action = match relation.topology_argument {
                Some(arg) => Some(matches!(
                    arg.kind,
                    ArgKind::LogicalAction { .. } | ArgKind::PhysicalAction { .. }
                )),
                None if matches!(relation.target, declaration::Target::Lifecycle(_))
                    || name == "__startup"
                    || name == "__shutdown" =>
                {
                    Some(true)
                }
                None => None,
            };
            let Some(action) = action else {
                return Err(syn::Error::new_spanned(
                    reaction.model.code(),
                    format!(
                        "topology relationship `{name}` must name a declared scalar port or action"
                    ),
                ));
            };
            let list = if action {
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
        let mut transition = None;
        for relation in &reaction.relations {
            let name = match &relation.target {
                declaration::Target::Lifecycle(DescriptorLifecycle::Startup) => {
                    "__startup".to_owned()
                }
                declaration::Target::Lifecycle(DescriptorLifecycle::Shutdown) => {
                    "__shutdown".to_owned()
                }
                declaration::Target::Lifecycle(DescriptorLifecycle::Reset) => {
                    reset = true;
                    continue;
                }
                _ => scalar_name(
                    relation
                        .reference
                        .expect("local topology relationship has reference syntax"),
                )?,
            };
            match relation.kind {
                Kind::Trigger => record(
                    name,
                    relation,
                    quote!(#compiler::ReactionRelationFlags::TRIGGER | #compiler::ReactionRelationFlags::USE),
                )?,
                Kind::Use => record(
                    name,
                    relation,
                    quote!(#compiler::ReactionRelationFlags::USE),
                )?,
                Kind::Effect => record(
                    name,
                    relation,
                    quote!(#compiler::ReactionRelationFlags::EFFECT),
                )?,
                Kind::Mode => {
                    let reference = relation
                        .reference
                        .expect("mode effect has reference syntax");
                    if !matches!(relation.target, declaration::Target::Mode(_)) {
                        return Err(syn::Error::new_spanned(
                            reference,
                            "reset/history effect requires a declared mode",
                        ));
                    }
                    let history = relation.transition == Some(ModeTransitionKind::History);
                    if transition
                        .as_ref()
                        .is_some_and(|previous| previous != &(name.clone(), history))
                    {
                        return Err(syn::Error::new_spanned(
                            reference,
                            "topology reaction has inconsistent mode transitions",
                        ));
                    }
                    transition = Some((name, history));
                }
                Kind::Scope => unreachable!("normalized scopes belong to reactions"),
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
                syn::Error::new_spanned(
                    reaction.model.code(),
                    "reset trigger requires a mode scope",
                )
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
    fn constructor_limits_preserve_supported_names() {
        let args: ReactorArgs =
            syn::parse_str("contract = \"test\", contract_version = 1").unwrap();
        // Mode names do not hide topology trigger and use targets in the port namespace.
        let model: Model = syn::parse_str(
            "fn Root(#[input] Active: u32) { reaction! { (Active) Active {} } mode! { initial Active {} } }",
        )
        .unwrap();
        assert!(generate(&args, &declaration::Declaration::new(&args, &model)).is_ok());
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
            assert!(
                generate(&args, &declaration::Declaration::new(&args, &model))
                    .unwrap_err()
                    .to_string()
                    .contains(expected)
            );
        }
    }
}
