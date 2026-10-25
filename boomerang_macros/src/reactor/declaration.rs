//! Shared declaration facts for metadata, typed topology, and payload emission.
//!
//! Normalize reaction identities, scopes, targets, and transition semantics once per expansion.
//! Retain Rust syntax for typed emission and diagnostics. Keep emitter-specific limits deferred:
//! an unsupported topology constructor must not reject a valid dynamic or payload declaration.
//!
//! Preserve descriptor positions within each relationship list. Topology emission must retain its
//! separate action and port first-encounter positions and last-declaration relation flags.
//! Structural names resolve modes before ports and actions. Payload arguments retain the first
//! same-name Rust argument so normalization does not change existing payload diagnostics.
//! Topology relationships retain the last same-name port or action independently of mode names.
//! Preserve this precedence for the topology lifecycle names `__startup` and `__shutdown` too.
//! Resolve initializer action names and types once; report cached errors at the existing emitter gates.
//!
//! Generated `ComponentDefinition` values already instantiate relative structure under each component
//! identity. Keep that interface and its typed port handles; do not add a second topology template.
use super::*;
use crate::reaction::{EffectType, PathOrIdent, TriggerType};
use boomerang_builder::{DescriptorLifecycle, DescriptorRelationshipKind, ModeTransitionKind};
use std::collections::BTreeMap;

/// Borrowed structural declarations shared by the component emitters.
pub(super) struct Declaration<'a> {
    /// Parsed Rust declaration, including types, banks, timing, and generic parameters.
    pub model: &'a Model,
    /// Modes in source declaration order.
    pub modes: Vec<&'a ModeDecl>,
    /// Reactions in source order, including reactions inside modes.
    pub reactions: Vec<Reaction<'a>>,
    /// First arbitrary builder statement, rejected only by structural emitters.
    pub unsupported: Option<Span>,
    /// Initializer action names and types in declaration order, with deferred validation errors.
    initializer_actions: syn::Result<Vec<(&'a Ident, &'a Type)>>,
    /// First duplicate-name diagnostic, deferred until structural validation is required.
    name_error: Option<syn::Error>,
}

/// One reaction with its canonical identity and resolved relationships.
pub(super) struct Reaction<'a> {
    /// Parsed reaction body used by typed payload and dynamic emission.
    pub model: &'a crate::reaction::Model,
    /// Canonical named or anonymous descriptor-local identity.
    pub slot: CanonicalReactionSlot,
    /// Enclosing mode, if this reaction has a mode scope.
    pub scope: Option<&'a ModeDecl>,
    /// Trigger, use, and effect relationships in payload argument order.
    pub relations: Vec<Relation<'a>>,
}

/// One declared relationship, before emitter-specific ABI ordering.
pub(super) struct Relation<'a> {
    /// Semantic relationship category, including mode effects.
    pub kind: DescriptorRelationshipKind,
    /// Resolved local declaration or lifecycle target.
    pub target: Target<'a>,
    /// Last same-name port or action, independent of descriptor and payload name precedence.
    pub topology_argument: Option<&'a Arg>,
    /// Original reference syntax for local paths and precise diagnostics.
    pub reference: Option<&'a PathOrIdent>,
    /// Reset or history semantics for a declared mode transition.
    pub transition: Option<ModeTransitionKind>,
    /// Position within its trigger, use, or effect declaration list.
    pub position: u32,
}

/// Shared target classification with concrete Rust declaration ownership.
pub(super) enum Target<'a> {
    /// Same-name arguments in the structural and payload namespaces.
    Argument {
        /// Structural target with port-before-action precedence over state and parameters.
        structural: &'a Arg,
        /// First Rust argument with this name, retained for payload diagnostic compatibility.
        payload: &'a Arg,
    },
    /// Declared mode with transition capability.
    Mode(&'a ModeDecl),
    /// Startup, shutdown, or reset lifecycle declaration.
    Lifecycle(DescriptorLifecycle),
    /// Child path or name without a local declaration.
    Lexical,
}

impl<'a> Declaration<'a> {
    /// Normalizes shared facts without imposing metadata or topology availability.
    pub fn new(args: &ReactorArgs, model: &'a Model) -> Self {
        let modes = model.body.modes().collect::<Vec<_>>();
        let mut targets = BTreeMap::new();
        let mut topology_targets = BTreeMap::new();
        let priority = |arg: &Arg| match arg.kind {
            ArgKind::Input { .. } | ArgKind::Output { .. } => 0,
            ArgKind::LogicalAction { .. } | ArgKind::PhysicalAction { .. } => 1,
            ArgKind::State { .. } | ArgKind::Param { .. } => 2,
        };
        for arg in &model.args {
            if priority(arg) < 2 {
                topology_targets.insert(ident_text(&arg.name.ident), arg);
            }
            targets
                .entry(ident_text(&arg.name.ident))
                .and_modify(|target| {
                    if let Target::Argument { structural, .. } = target {
                        if priority(arg) < priority(structural) {
                            *structural = arg;
                        }
                    }
                })
                .or_insert(Target::Argument {
                    structural: arg,
                    payload: arg,
                });
        }
        for mode in &modes {
            targets.insert(mode.name_str(), Target::Mode(mode));
        }
        let resolve = |reference: &'a PathOrIdent| match reference {
            PathOrIdent::Simple(ident) => match targets.get(&ident_text(ident)) {
                Some(Target::Argument {
                    structural,
                    payload,
                }) => Target::Argument {
                    structural,
                    payload,
                },
                Some(Target::Mode(mode)) => Target::Mode(mode),
                _ => Target::Lexical,
            },
            PathOrIdent::Field(_) => Target::Lexical,
        };
        let mut declarations = Vec::new();
        model.body.reactions(None, &mut declarations);
        let mut next_generated = 0;
        let mut names = BTreeSet::new();
        let mut name_error = None;
        let reactions = declarations
            .into_iter()
            .map(|(scope, reaction)| {
                if let Some(name) = reaction.name() {
                    if !names.insert(ident_text(name)) && name_error.is_none() {
                        name_error = Some(syn::Error::new(
                            name.span(),
                            format!("duplicate reaction name `{name}`"),
                        ));
                    }
                }
                let mut relations = Vec::new();
                let mut record = |kind,
                                  target,
                                  reference: Option<&'a PathOrIdent>,
                                  transition,
                                  position: usize| {
                    let topology_name = match reference {
                        Some(PathOrIdent::Simple(ident)) => Some(ident_text(ident)),
                        None => match target {
                            Target::Lifecycle(DescriptorLifecycle::Startup) => {
                                Some("__startup".to_owned())
                            }
                            Target::Lifecycle(DescriptorLifecycle::Shutdown) => {
                                Some("__shutdown".to_owned())
                            }
                            _ => None,
                        },
                        Some(PathOrIdent::Field(_)) => None,
                    };
                    relations.push(Relation {
                        kind,
                        target,
                        topology_argument: topology_name
                            .and_then(|name| topology_targets.get(&name).copied()),
                        reference,
                        transition,
                        position: u32::try_from(position).expect("relationship count fits u32"),
                    });
                };
                for (position, trigger) in reaction.triggers().iter().enumerate() {
                    let (target, reference) = match trigger {
                        TriggerType::Startup => {
                            (Target::Lifecycle(DescriptorLifecycle::Startup), None)
                        }
                        TriggerType::Shutdown => {
                            (Target::Lifecycle(DescriptorLifecycle::Shutdown), None)
                        }
                        TriggerType::Reset => (Target::Lifecycle(DescriptorLifecycle::Reset), None),
                        TriggerType::Regular(reference) => (resolve(reference), Some(reference)),
                    };
                    record(
                        DescriptorRelationshipKind::Trigger,
                        target,
                        reference,
                        None,
                        position,
                    );
                }
                for (position, reference) in reaction.uses().iter().enumerate() {
                    record(
                        DescriptorRelationshipKind::Use,
                        resolve(reference),
                        Some(reference),
                        None,
                        position,
                    );
                }
                for (position, effect) in reaction.effects().iter().enumerate() {
                    let reference = match effect {
                        EffectType::Regular(path)
                        | EffectType::Reset(path)
                        | EffectType::History(path) => path,
                    };
                    let target = resolve(reference);
                    let transition = match effect {
                        EffectType::History(_) => Some(ModeTransitionKind::History),
                        EffectType::Reset(_) => Some(ModeTransitionKind::Reset),
                        EffectType::Regular(_) if matches!(target, Target::Mode(_)) => {
                            Some(ModeTransitionKind::Reset)
                        }
                        EffectType::Regular(_) => None,
                    };
                    let kind = if transition.is_some() {
                        DescriptorRelationshipKind::Mode
                    } else {
                        DescriptorRelationshipKind::Effect
                    };
                    record(kind, target, Some(reference), transition, position);
                }
                Reaction {
                    model: reaction,
                    slot: CanonicalReactionSlot::for_reaction(reaction, &mut next_generated),
                    scope,
                    relations,
                }
            })
            .collect();
        names.clear();
        for mode in &modes {
            if !names.insert(mode.name_str()) && name_error.is_none() {
                name_error = Some(syn::Error::new(
                    mode.name.span(),
                    format!("duplicate mode name `{}`", mode.name),
                ));
            }
        }
        Self {
            model,
            modes,
            reactions,
            unsupported: model.body.unsupported_span(),
            initializer_actions: args
                .init_context
                .as_ref()
                .map_or_else(|| Ok(Vec::new()), |context| context.actions(model)),
            name_error,
        }
    }

    /// Returns initializer action declarations without repeating name lookup or validation.
    pub fn initializer_actions(&self) -> syn::Result<&[(&'a Ident, &'a Type)]> {
        self.initializer_actions.as_deref().map_err(Clone::clone)
    }

    /// Reports the shared name error when an emitter requires structural declarations.
    pub fn validate_names(&self) -> syn::Result<()> {
        match &self.name_error {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }
}
