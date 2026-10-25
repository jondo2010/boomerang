//! The declaration's compatibility semantics, independent of emitted Rust and implementation bodies.
//!
//! Build the canonical descriptor once on the proc-macro host. Metadata renders these records;
//! native payloads embed the fingerprint produced by the builder's existing encoder.
use super::*;
use boomerang_builder::{
    compiler::{ContractId, StablePath, StablePathSegment},
    ActionSlot, ActionSlotId, ComponentDescriptor, DescriptorBound, DescriptorBounds,
    DescriptorLifecycle, DescriptorRelationship, DescriptorRelationshipKind as Kind,
    DescriptorRelationshipTarget as Target, InitializerCapabilities, ModeSlot, ModeSlotId,
    ModeTransitionKind, PortDirection, PortSlot, PortSlotId, ReactionSlot, ReactionSlotId,
    ReactorSlot, ReactorSlotId, StateSlot, StateSlotId,
};

pub(super) fn parse_descriptor(
    args: &ReactorArgs,
    declaration: &declaration::Declaration<'_>,
) -> syn::Result<Option<ComponentDescriptor>> {
    let model = declaration.model;
    args.validate()?;
    let (Some(contract), Some(version)) = (&args.contract, &args.contract_version) else {
        return Ok(None);
    };
    let contract_id = ContractId::new(contract.value()).map_err(|error| {
            syn::Error::new(contract.span(), format!("contract must be non-empty, contain no control characters, and have no surrounding whitespace: {error}"))
        })?;
    let version = version
        .base10_parse::<u64>()
        .map_err(|_| syn::Error::new(version.span(), "contract_version must fit in u64"))?;
    let bounds = args.descriptor_bounds()?;
    declaration.validate_names()?;
    if let Some(span) = declaration.unsupported {
        return Err(syn::Error::new(
            span,
            "deployment descriptor requires reaction! syntax",
        ));
    }
    let root = StablePath::from_name(ident_text(&model.name))
        .map_err(|error| syn::Error::new(model.name.span(), error))?;
    let reactor = ReactorSlotId::from_path(root.clone());
    let path = |name: &str| {
        root.append_name(name)
            .expect("Rust identifiers are valid stable names")
    };
    let capabilities = args
        .init_context
        .as_ref()
        .map(|context| {
            Ok::<_, syn::Error>(InitializerCapabilities {
                source: context.source.as_ref().map(syn::LitStr::value),
                actions: declaration
                    .initializer_actions()?
                    .iter()
                    .map(|(name, _)| ActionSlotId::from_path(path(&ident_text(name))))
                    .collect(),
                clock_control: context.clock_control(),
            })
        })
        .transpose()?;
    let mut ports = Vec::new();
    let mut actions = Vec::new();
    let mut states = Vec::new();
    for arg in &model.args {
        let slot = path(&ident_text(&arg.name.ident));
        match arg.kind {
            ArgKind::Input { .. } | ArgKind::Output { .. } => ports.push(PortSlot {
                id: PortSlotId::from_path(slot),
                reactor: reactor.clone(),
                direction: if matches!(arg.kind, ArgKind::Input { .. }) {
                    PortDirection::Input
                } else {
                    PortDirection::Output
                },
            }),
            ArgKind::LogicalAction { .. } | ArgKind::PhysicalAction { .. } => {
                actions.push(ActionSlot {
                    id: ActionSlotId::from_path(slot),
                    reactor: reactor.clone(),
                })
            }
            ArgKind::State { .. } => states.push(StateSlot {
                id: StateSlotId::from_path(slot),
                reactor: reactor.clone(),
            }),
            ArgKind::Param { .. } => {}
        }
    }
    let modes = declaration
        .modes
        .iter()
        .map(|mode| ModeSlot {
            id: ModeSlotId::from_path(path(&ident_text(&mode.name))),
            reactor: reactor.clone(),
            parent: None,
            initial: mode.initial,
        })
        .collect::<Vec<_>>();
    let mut reactions = Vec::new();
    let mut relationships = Vec::new();
    for reaction in &declaration.reactions {
        let slot = match &reaction.slot {
            CanonicalReactionSlot::Named(name) => path(name),
            CanonicalReactionSlot::Generated(ordinal) => root.append_generated_ordinal(*ordinal),
        };
        let reaction_id = ReactionSlotId::from_path(slot);
        reactions.push(ReactionSlot {
            id: reaction_id.clone(),
            reactor: reactor.clone(),
        });
        for relation in &reaction.relations {
            let target_path = || {
                path_segments(
                    relation
                        .reference
                        .expect("local relationship has reference syntax"),
                )
                .iter()
                .fold(root.clone(), |path, name| {
                    path.append_name(name.as_str())
                        .expect("valid relationship path")
                })
            };
            let target = match &relation.target {
                declaration::Target::Lifecycle(lifecycle) => Target::Lifecycle(*lifecycle),
                declaration::Target::Mode(mode) => {
                    Target::Mode(ModeSlotId::from_path(path(&mode.name_str())))
                }
                declaration::Target::Argument {
                    structural: arg, ..
                } => match arg.kind {
                    ArgKind::Input { .. } | ArgKind::Output { .. } => {
                        Target::Port(PortSlotId::from_path(target_path()))
                    }
                    ArgKind::LogicalAction { .. } | ArgKind::PhysicalAction { .. } => {
                        Target::Action(ActionSlotId::from_path(target_path()))
                    }
                    ArgKind::State { .. } | ArgKind::Param { .. } => Target::Lexical(target_path()),
                },
                declaration::Target::Lexical => Target::Lexical(target_path()),
            };
            relationships.push(DescriptorRelationship {
                reaction: reaction_id.clone(),
                kind: relation.kind,
                target,
                mode_transition: relation.transition,
                declaration_position: relation.position,
            });
        }
        if let Some(scope) = reaction.scope {
            relationships.push(DescriptorRelationship {
                reaction: reaction_id,
                kind: Kind::Scope,
                target: Target::Mode(ModeSlotId::from_path(path(&scope.name_str()))),
                mode_transition: None,
                declaration_position: 0,
            });
        }
    }
    ComponentDescriptor::try_new(
        contract_id,
        version,
        boomerang_runtime::binding::COMPONENT_DESCRIPTOR_MACRO_ABI,
        vec![ReactorSlot {
            id: reactor,
            parent: None,
            capabilities,
        }],
        ports,
        actions,
        reactions,
        modes,
        states,
        vec![],
        relationships,
        vec![],
        vec![],
        DescriptorBounds {
            queue_capacity: DescriptorBound::Known(bounds.queue_capacity),
            payload_bytes: DescriptorBound::Known(bounds.payload_bytes),
            state_bytes: DescriptorBound::Known(bounds.state_bytes),
            scratch_bytes: DescriptorBound::Known(bounds.scratch_bytes),
        },
    )
    .map(Some)
    .map_err(|error| syn::Error::new(model.name.span(), error))
}

/// Render the same typed records that feed canonical fingerprinting. This is Rust emission,
/// not a second compatibility encoding; generated metadata still uses descriptor validation.
pub(super) fn emit_descriptor(descriptor: &ComponentDescriptor) -> TokenStream {
    let contract = descriptor.contract_id().as_str();
    let version = descriptor.contract_version();
    let abi = descriptor.macro_abi();
    let reactors = descriptor.reactor_slots().iter().map(|slot| {
            let id = path_tokens(slot.id.path());
            let capabilities = slot.capabilities.as_ref().map_or_else(|| quote!(None), |context| {
                let source = context.source.as_ref().map_or_else(|| quote!(None), |source| quote!(Some(#source.into())));
                let actions = context.actions.iter().map(|action| { let path = path_tokens(action.path()); quote!(::boomerang::builder::ActionSlotId::from_path(#path)) });
                let clock = context.clock_control;
                quote!(Some(::boomerang::builder::InitializerCapabilities { source: #source, actions: vec![#(#actions),*], clock_control: #clock }))
            });
            quote!(::boomerang::builder::ReactorSlot { id: ::boomerang::builder::ReactorSlotId::from_path(#id), parent: None, capabilities: #capabilities })
        });
    let ports = descriptor.port_slots().iter().map(|slot| {
            let id = path_tokens(slot.id.path()); let reactor = path_tokens(slot.reactor.path());
            let direction = match slot.direction { PortDirection::Input => quote!(Input), PortDirection::Output => quote!(Output) };
            quote!(::boomerang::builder::PortSlot { id: ::boomerang::builder::PortSlotId::from_path(#id), reactor: ::boomerang::builder::ReactorSlotId::from_path(#reactor), direction: ::boomerang::builder::PortDirection::#direction })
        });
    macro_rules! owned_slots {
            ($slots:expr, $slot:ident, $id:ident) => {
                $slots.iter().map(|slot| {
                    let id = path_tokens(slot.id.path()); let reactor = path_tokens(slot.reactor.path());
                    quote!(::boomerang::builder::$slot { id: ::boomerang::builder::$id::from_path(#id), reactor: ::boomerang::builder::ReactorSlotId::from_path(#reactor) })
                })
            };
        }
    let actions = owned_slots!(descriptor.action_slots(), ActionSlot, ActionSlotId);
    let reactions = owned_slots!(descriptor.reaction_slots(), ReactionSlot, ReactionSlotId);
    let states = owned_slots!(descriptor.state_slots(), StateSlot, StateSlotId);
    let modes = descriptor.mode_slots().iter().map(|slot| {
            let id = path_tokens(slot.id.path()); let reactor = path_tokens(slot.reactor.path()); let initial = slot.initial;
            quote!(::boomerang::builder::ModeSlot { id: ::boomerang::builder::ModeSlotId::from_path(#id), reactor: ::boomerang::builder::ReactorSlotId::from_path(#reactor), parent: None, initial: #initial })
        });
    let relationships = descriptor.relationships().iter().map(|relationship| {
            let reaction = path_tokens(relationship.reaction.path());
            let kind = match relationship.kind { Kind::Trigger => quote!(Trigger), Kind::Use => quote!(Use), Kind::Effect => quote!(Effect), Kind::Mode => quote!(Mode), Kind::Scope => quote!(Scope) };
            let target = match &relationship.target {
                Target::Port(id) => { let path = path_tokens(id.path()); quote!(::boomerang::builder::DescriptorRelationshipTarget::Port(::boomerang::builder::PortSlotId::from_path(#path))) },
                Target::Action(id) => { let path = path_tokens(id.path()); quote!(::boomerang::builder::DescriptorRelationshipTarget::Action(::boomerang::builder::ActionSlotId::from_path(#path))) },
                Target::Mode(id) => { let path = path_tokens(id.path()); quote!(::boomerang::builder::DescriptorRelationshipTarget::Mode(::boomerang::builder::ModeSlotId::from_path(#path))) },
                Target::Lexical(path) => { let path = path_tokens(path); quote!(::boomerang::builder::DescriptorRelationshipTarget::Lexical(#path)) },
                Target::Lifecycle(lifecycle) => {
                    let lifecycle = match lifecycle { DescriptorLifecycle::Startup => quote!(Startup), DescriptorLifecycle::Shutdown => quote!(Shutdown), DescriptorLifecycle::Reset => quote!(Reset) };
                    quote!(::boomerang::builder::DescriptorRelationshipTarget::Lifecycle(::boomerang::builder::DescriptorLifecycle::#lifecycle))
                }
            };
            let transition = match relationship.mode_transition {
                None => quote!(None), Some(ModeTransitionKind::Reset) => quote!(Some(::boomerang::builder::ModeTransitionKind::Reset)), Some(ModeTransitionKind::History) => quote!(Some(::boomerang::builder::ModeTransitionKind::History)),
            };
            let position = relationship.declaration_position;
            quote!(::boomerang::builder::DescriptorRelationship { reaction: ::boomerang::builder::ReactionSlotId::from_path(#reaction), kind: ::boomerang::builder::DescriptorRelationshipKind::#kind, target: #target, mode_transition: #transition, declaration_position: #position })
        });
    let known = |bound| match bound {
        DescriptorBound::Known(value) => value,
        DescriptorBound::Unknown => unreachable!("macro bounds are known"),
    };
    let bounds = descriptor.bounds();
    let queue = known(bounds.queue_capacity);
    let payload = known(bounds.payload_bytes);
    let state = known(bounds.state_bytes);
    let scratch = known(bounds.scratch_bytes);
    quote! {
        /// Returns the canonical host-owned component descriptor.
        pub fn descriptor() -> ::boomerang::builder::ComponentDescriptor {
            ::boomerang::builder::ComponentDescriptor::__from_macro(#contract, #version, #abi,
                vec![#(#reactors),*], vec![#(#ports),*], vec![#(#actions),*], vec![#(#reactions),*], vec![#(#modes),*], vec![#(#states),*], vec![], vec![#(#relationships),*], vec![], vec![],
                ::boomerang::builder::DescriptorBounds { queue_capacity: ::boomerang::builder::DescriptorBound::Known(#queue), payload_bytes: ::boomerang::builder::DescriptorBound::Known(#payload), state_bytes: ::boomerang::builder::DescriptorBound::Known(#state), scratch_bytes: ::boomerang::builder::DescriptorBound::Known(#scratch) })
        }
    }
}

fn path_tokens(path: &StablePath) -> TokenStream {
    let mut segments = path.segments().iter();
    let Some(StablePathSegment::Name(first)) = segments.next() else {
        unreachable!("component paths start with a name")
    };
    let mut output = quote!(::boomerang::builder::compiler::StablePath::from_name(#first).expect("valid descriptor name"));
    for segment in segments {
        output = match segment {
            StablePathSegment::Name(name) => {
                quote!(#output.append_name(#name).expect("valid descriptor name"))
            }
            StablePathSegment::BankIndex(index) => quote!(#output.append_bank_index(#index)),
            StablePathSegment::GeneratedOrdinal(ordinal) => {
                quote!(#output.append_generated_ordinal(#ordinal))
            }
        };
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    const ARGS: &str = r#"contract = "example.controller", contract_version = 1,
        bounds(queue_capacity = 16, payload_bytes = 1024, state_bytes = 512, scratch_bytes = 256)"#;
    const MODEL: &str = r#"pub fn Root(#[input] left: u32, #[input] right: u32,
        #[output] command: u32, #[state(default = 1)] count: u32,
        #[physical_action] sensor: u32) -> impl Reactor {
        reaction! { control(left, right) -> command { *command = Some(state.count); } }
        mode! { initial idle { reaction! { (reset) {} } } }
        mode! { active {} }
    }"#;

    fn descriptor(args: &str, model: &str) -> ComponentDescriptor {
        let args = syn::parse_str(args).unwrap();
        parse_descriptor(
            &args,
            &declaration::Declaration::new(&args, &syn::parse_str(model).unwrap()),
        )
        .unwrap()
        .unwrap()
    }

    #[test]
    fn implementation_bodies_initializers_and_formatting_do_not_affect_identity() {
        let expected = descriptor(ARGS, MODEL).fingerprint();
        for changed in [
            MODEL.replace("Some(state.count)", "Some(999)"),
            MODEL.replace("default = 1", "default = host_unavailable_helper()"),
            MODEL.replace("control(left, right)", "control ( left , right )"),
        ] {
            assert_eq!(expected, descriptor(ARGS, &changed).fingerprint());
        }
    }

    #[test]
    fn contract_version_bounds_direction_modes_and_relationship_order_affect_identity() {
        let shared_name = descriptor(
            ARGS,
            "fn Root(#[state] value: u32, #[input] value: u32) { reaction! { observe(value) {} } }",
        );
        assert!(matches!(shared_name.relationships()[0].target,
            Target::Port(ref slot) if slot.path().to_string() == "Root/value"));
        let expected = descriptor(ARGS, MODEL).fingerprint();
        for changed in [
            ARGS.replace("example.controller", "example.other"),
            ARGS.replace("contract_version = 1", "contract_version = 2"),
            ARGS.replace("queue_capacity = 16", "queue_capacity = 17"),
            ARGS.replace("payload_bytes = 1024", "payload_bytes = 1025"),
            ARGS.replace("state_bytes = 512", "state_bytes = 513"),
            ARGS.replace("scratch_bytes = 256", "scratch_bytes = 257"),
        ] {
            assert_ne!(
                expected,
                descriptor(&changed, MODEL).fingerprint(),
                "{changed}"
            );
        }
        for changed in [
            MODEL.replace("#[input] left", "#[output] left"),
            MODEL.replace("control(left, right)", "control(right, left)"),
            MODEL.replace("-> command {", "-> right {"),
            MODEL.replace("mode! { active", "mode! { other"),
            MODEL
                .replace("initial idle", "idle")
                .replace("mode! { active", "mode! { initial active"),
            MODEL.replace("(reset) {}", "(reset) -> history(active) {}"),
            MODEL.replace("control(left, right)", "(left, right)"),
            MODEL.replace("count: u32", "counter: u32"),
        ] {
            assert_ne!(
                expected,
                descriptor(ARGS, &changed).fingerprint(),
                "{changed}"
            );
        }
    }

    #[test]
    fn initializer_capability_source_actions_order_and_clock_authority_affect_identity() {
        let model = "pub fn Root(#[physical_action] first: u32, #[physical_action] second: u32) -> impl Reactor { reaction! { start(startup) {} } }";
        let args = format!("{ARGS}, state = State, state_init = initialize");
        let expected = descriptor(&args, model).fingerprint();
        let capabilities = format!("{args}, init_context(name = Init, source = \"network\", actions = [first, second], clock_control = false)");
        let capable = descriptor(&capabilities, model).fingerprint();
        assert_ne!(expected, capable);
        for changed in [
            capabilities.replace("network", "sensor"),
            capabilities.replace("[first, second]", "[second, first]"),
            capabilities.replace("[first, second]", "[first]"),
            capabilities.replace("clock_control = false", "clock_control = true"),
        ] {
            assert_ne!(
                capable,
                descriptor(&changed, model).fingerprint(),
                "{changed}"
            );
        }
    }
}
