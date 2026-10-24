use boomerang_builder::compiler::FederateSlice;
use boomerang_runtime::{
    image::{
        ActionTiming, BankInfoImage, BindingKind, EnclaveImage, LevelReactionImage,
        LifecycleReactionImage, RouteDirection, StorageBounds, TimerStartupImage, TimingDomain,
    },
    TransitionKind,
};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use tinymap::{IndexSpan, SliceRange};

pub(super) fn render_enclave_image(index: usize, image: &EnclaveImage<'_>) -> TokenStream {
    let prefix = format!("E{index}");
    let mut tokens = TokenStream::new();
    tokens.extend(render_reactors(&prefix, image));
    tokens.extend(render_actions(&prefix, image));
    tokens.extend(render_ports(&prefix, image));
    tokens.extend(render_reactions(&prefix, image));
    tokens.extend(render_modes(&prefix, image));
    tokens.extend(render_scopes(&prefix, image));
    tokens.extend(render_level_reactions(
        &format!("{prefix}_REACTION_TRIGGERS"),
        image.reaction_triggers,
    ));
    tokens.extend(render_indices(
        &format!("{prefix}_REACTION_USE_PORTS"),
        format_ident!("PortIndex"),
        image.reaction_use_ports.iter().map(|value| value.as_u32()),
    ));
    tokens.extend(render_indices(
        &format!("{prefix}_REACTION_EFFECT_PORTS"),
        format_ident!("PortIndex"),
        image
            .reaction_effect_ports
            .iter()
            .map(|value| value.as_u32()),
    ));
    tokens.extend(render_indices(
        &format!("{prefix}_REACTION_ACTIONS"),
        format_ident!("ActionIndex"),
        image.reaction_actions.iter().map(|value| value.as_u32()),
    ));
    tokens.extend(render_indices(
        &format!("{prefix}_REACTION_MODES"),
        format_ident!("ModeIndex"),
        image.reaction_modes.iter().map(|value| value.as_u32()),
    ));
    tokens.extend(render_indices(
        &format!("{prefix}_SCOPE_DESCENDANTS"),
        format_ident!("ScopeIndex"),
        image.scope_descendants.iter().map(|value| value.as_u32()),
    ));
    tokens.extend(render_indices(
        &format!("{prefix}_SCOPE_LOGICAL_ACTIONS"),
        format_ident!("ActionIndex"),
        image
            .scope_logical_actions
            .iter()
            .map(|value| value.as_u32()),
    ));
    tokens.extend(render_timer_startups(
        &format!("{prefix}_SCOPE_TIMER_STARTUPS"),
        image.scope_timer_startups,
    ));
    tokens.extend(render_level_reactions(
        &format!("{prefix}_SCOPE_RESET_REACTIONS"),
        image.scope_reset_reactions,
    ));
    tokens.extend(render_lifecycle_reactions(
        &format!("{prefix}_SCOPE_STARTUP_REACTIONS"),
        image.scope_startup_reactions,
    ));
    tokens.extend(render_lifecycle_reactions(
        &format!("{prefix}_SCOPE_SHUTDOWN_REACTIONS"),
        image.scope_shutdown_reactions,
    ));
    tokens.extend(render_timer_startups(
        &format!("{prefix}_STARTUP_ACTIONS"),
        image.startup_actions,
    ));
    tokens.extend(render_timer_startups(
        &format!("{prefix}_TIMER_STARTUP_ACTIONS"),
        image.timer_startup_actions,
    ));
    tokens.extend(render_lifecycle_reactions(
        &format!("{prefix}_SHUTDOWN_REACTIONS"),
        image.shutdown_reactions,
    ));
    tokens.extend(render_indices(
        &format!("{prefix}_SHUTDOWN_ACTIONS"),
        format_ident!("ActionIndex"),
        image.shutdown_actions.iter().map(|value| value.as_u32()),
    ));
    tokens.extend(render_routes(&prefix, image));
    tokens.extend(render_required_bindings(&prefix, image));
    let image_name = format_ident!("{prefix}_IMAGE");
    let enclave_id = image.enclave_id.as_str();
    let bounds = storage_bounds(image.storage_bounds);
    let reactors = format_ident!("{prefix}_REACTORS");
    let actions = format_ident!("{prefix}_ACTIONS");
    let ports = format_ident!("{prefix}_PORTS");
    let reactions = format_ident!("{prefix}_REACTIONS");
    let modes = format_ident!("{prefix}_MODES");
    let scopes = format_ident!("{prefix}_SCOPES");
    let reaction_triggers = format_ident!("{prefix}_REACTION_TRIGGERS");
    let reaction_use_ports = format_ident!("{prefix}_REACTION_USE_PORTS");
    let reaction_effect_ports = format_ident!("{prefix}_REACTION_EFFECT_PORTS");
    let reaction_actions = format_ident!("{prefix}_REACTION_ACTIONS");
    let reaction_modes = format_ident!("{prefix}_REACTION_MODES");
    let scope_descendants = format_ident!("{prefix}_SCOPE_DESCENDANTS");
    let scope_logical_actions = format_ident!("{prefix}_SCOPE_LOGICAL_ACTIONS");
    let scope_timer_startups = format_ident!("{prefix}_SCOPE_TIMER_STARTUPS");
    let scope_reset_reactions = format_ident!("{prefix}_SCOPE_RESET_REACTIONS");
    let scope_startup_reactions = format_ident!("{prefix}_SCOPE_STARTUP_REACTIONS");
    let scope_shutdown_reactions = format_ident!("{prefix}_SCOPE_SHUTDOWN_REACTIONS");
    let startup_actions = format_ident!("{prefix}_STARTUP_ACTIONS");
    let timer_startup_actions = format_ident!("{prefix}_TIMER_STARTUP_ACTIONS");
    let shutdown_reactions = format_ident!("{prefix}_SHUTDOWN_REACTIONS");
    let shutdown_actions = format_ident!("{prefix}_SHUTDOWN_ACTIONS");
    let routes = format_ident!("{prefix}_ROUTES");
    let required_bindings = format_ident!("{prefix}_REQUIRED_BINDINGS");
    tokens.extend(quote! {
        const #image_name: EnclaveImage<'static> = EnclaveImage {
            enclave_id: EnclaveId::new(#enclave_id),
            reactors: TinyMapRef::from_slice(&#reactors),
            actions: TinyMapRef::from_slice(&#actions),
            ports: TinyMapRef::from_slice(&#ports),
            reactions: TinyMapRef::from_slice(&#reactions),
            modes: TinyMapRef::from_slice(&#modes),
            scopes: TinyMapRef::from_slice(&#scopes),
            reaction_triggers: &#reaction_triggers,
            reaction_use_ports: &#reaction_use_ports,
            reaction_effect_ports: &#reaction_effect_ports,
            reaction_actions: &#reaction_actions,
            reaction_modes: &#reaction_modes,
            scope_descendants: &#scope_descendants,
            scope_logical_actions: &#scope_logical_actions,
            scope_timer_startups: &#scope_timer_startups,
            scope_reset_reactions: &#scope_reset_reactions,
            scope_startup_reactions: &#scope_startup_reactions,
            scope_shutdown_reactions: &#scope_shutdown_reactions,
            startup_actions: &#startup_actions,
            timer_startup_actions: &#timer_startup_actions,
            shutdown_reactions: &#shutdown_reactions,
            shutdown_actions: &#shutdown_actions,
            routes: TinyMapRef::from_slice(&#routes),
            required_bindings: TinyMapRef::from_slice(&#required_bindings),
            storage_bounds: &#bounds,
        };
    });
    tokens
}

/// Emits the selected Federate descriptor and its raw Enclave descriptor table.
pub(super) fn render_federate(slice: &FederateSlice<'_>) -> TokenStream {
    let enclave_count = slice.enclaves().len();
    let enclave_len = proc_macro2::Literal::usize_unsuffixed(enclave_count);
    let enclave_images = (0..enclave_count).map(|index| format_ident!("E{index}_IMAGE"));
    let federate = proc_macro2::Literal::u32_unsuffixed(slice.federate().as_u32());
    let id = slice.id().as_str();
    let target = slice.target().as_str();
    let runtime = slice.runtime().as_str();
    let enclave_range = index_span(slice.enclave_range());
    quote! {
        static ENCLAVE_IMAGES: [EnclaveImage<'static>; #enclave_len] = [#(#enclave_images),*];
        pub static FEDERATE: FederateIndex = FederateIndex::new(#federate);
        const FEDERATE_IMAGE: FederateImage<'static> = FederateImage::new(
            FederateId::new(#id),
            TargetId::new(#target),
            RuntimeBackendId::new(#runtime),
            #enclave_range,
        );
        pub static FEDERATE_VIEW: FederateImageView<'static> =
            match FederateImageView::new(FEDERATE_IMAGE, &ENCLAVE_IMAGES) {
                Ok(view) => view,
                Err(_) => panic!("invalid generated Federate image"),
            };
    }
}
fn render_reactors(prefix: &str, image: &EnclaveImage<'_>) -> TokenStream {
    let name = format_ident!("{prefix}_REACTORS");
    let len = image.reactors.len();
    let values = image.reactors.values().map(|value| {
        let state_binding = value.state_binding().as_u32();
        let state_slot = value.state_slot().as_u32();
        let root_scope = value.root_scope().as_u32();
        let modes = index_span(value.modes());
        let initial_mode = optional_index(
            format_ident!("ModeIndex"),
            value.initial_mode().map(|value| value.as_u32()),
        );
        let bank = optional_bank(value.bank());
        quote!(ReactorImage::new(
            BindingSlotIndex::new(#state_binding),
            StateSlotIndex::new(#state_slot),
            ScopeIndex::new(#root_scope),
            #modes,
            #initial_mode,
            #bank,
        ))
    });
    quote!(static #name: [ReactorImage; #len] = [#(#values),*];)
}

/// Renders the immutable action table for one enclave.
fn render_actions(prefix: &str, image: &EnclaveImage<'_>) -> TokenStream {
    let name = format_ident!("{prefix}_ACTIONS");
    let len = image.actions.len();
    let values = image.actions.values().map(|value| {
        let scope = value.scope().as_u32();
        let storage_slot = value.storage_slot().as_u32();
        let timing = action_timing(value.timing());
        let triggers = slice_range(value.triggers());
        let binding = optional_index(
            format_ident!("BindingSlotIndex"),
            value.binding().map(|value| value.as_u32()),
        );
        quote!(ActionImage::new(
            ScopeIndex::new(#scope),
            ActionSlotIndex::new(#storage_slot),
            #timing,
            #triggers,
            #binding,
        ))
    });
    quote!(static #name: [ActionImage; #len] = [#(#values),*];)
}

/// Renders the immutable port table for one enclave.
fn render_ports(prefix: &str, image: &EnclaveImage<'_>) -> TokenStream {
    let name = format_ident!("{prefix}_PORTS");
    let len = image.ports.len();
    let values = image.ports.values().map(|value| {
        let scope = value.scope().as_u32();
        let triggers = slice_range(value.triggers());
        let binding = value.binding().as_u32();
        quote!(PortImage::new(
            ScopeIndex::new(#scope),
            #triggers,
            BindingSlotIndex::new(#binding),
        ))
    });
    quote!(static #name: [PortImage; #len] = [#(#values),*];)
}

/// Renders the immutable reaction table for one enclave.
fn render_reactions(prefix: &str, image: &EnclaveImage<'_>) -> TokenStream {
    let name = format_ident!("{prefix}_REACTIONS");
    let len = image.reactions.len();
    let values = image.reactions.values().map(|value| {
        let reactor = value.reactor().as_u32();
        let scope = value.scope().as_u32();
        let level = value.dependency_level();
        let binding = value.binding().as_u32();
        let use_ports = slice_range(value.use_ports());
        let effect_ports = slice_range(value.effect_ports());
        let actions = slice_range(value.actions());
        let enabled_modes = slice_range(value.enabled_modes());
        let base = quote!(ReactionImage::new(
            ReactorIndex::new(#reactor),
            ScopeIndex::new(#scope),
            #level,
            BindingSlotIndex::new(#binding),
            #use_ports,
            #effect_ports,
            #actions,
            #enabled_modes,
        ));
        value.mode_effect().map_or_else(
            || base.clone(),
            |effect| {
                let target = effect.target.as_u32();
                let transition = transition(effect.transition);
                quote!(#base.with_mode_effect(boomerang_runtime::CompiledModeEffectRef {
                    target: ModeIndex::new(#target),
                    transition: #transition,
                }))
            },
        )
    });
    quote!(static #name: [ReactionImage; #len] = [#(#values),*];)
}

/// Renders the immutable mode table for one enclave.
fn render_modes(prefix: &str, image: &EnclaveImage<'_>) -> TokenStream {
    let name = format_ident!("{prefix}_MODES");
    let len = image.modes.len();
    let values = image.modes.values().map(|value| {
        let reactor = value.reactor().as_u32();
        let scope = value.scope().as_u32();
        quote!(ModeImage::new(
            ReactorIndex::new(#reactor),
            ScopeIndex::new(#scope),
        ))
    });
    quote!(static #name: [ModeImage; #len] = [#(#values),*];)
}

/// Renders the immutable scope table for one enclave.
fn render_scopes(prefix: &str, image: &EnclaveImage<'_>) -> TokenStream {
    let name = format_ident!("{prefix}_SCOPES");
    let len = image.scopes.len();
    let values = image.scopes.values().map(|value| {
        let parent = optional_index(
            format_ident!("ScopeIndex"),
            value.parent().map(|value| value.as_u32()),
        );
        let reactor = value.reactor().as_u32();
        let mode = optional_index(
            format_ident!("ModeIndex"),
            value.mode().map(|value| value.as_u32()),
        );
        let descendants = slice_range(value.descendants());
        let logical_actions = slice_range(value.logical_actions());
        let timer_startups = slice_range(value.timer_startups());
        let reset_reactions = slice_range(value.reset_reactions());
        let startup_reactions = slice_range(value.startup_reactions());
        let shutdown_reactions = slice_range(value.shutdown_reactions());
        quote!(ScopeImage::new(
            #parent,
            ReactorIndex::new(#reactor),
            #mode,
            #descendants,
            #logical_actions,
            #timer_startups,
            #reset_reactions,
            #startup_reactions,
            #shutdown_reactions,
        ))
    });
    quote!(static #name: [ScopeImage; #len] = [#(#values),*];)
}

/// Renders the immutable route table for one enclave.
fn render_routes(prefix: &str, image: &EnclaveImage<'_>) -> TokenStream {
    let name = format_ident!("{prefix}_ROUTES");
    let len = image.routes.len();
    let values = image.routes.values().map(|value| {
        let boundary = value.boundary().as_str();
        let local_port = value.local_port().as_u32();
        let direction = route_direction(value.direction());
        let domain = timing_domain(value.timing_domain());
        let delay = value.delay_nanos();
        quote!(RouteImage::new(
            BoundaryId::new(#boundary),
            PortIndex::new(#local_port),
            #direction,
            #domain,
            #delay,
        ))
    });
    quote!(static #name: [RouteImage; #len] = [#(#values),*];)
}

/// Renders the immutable required-binding table for one enclave.
fn render_required_bindings(prefix: &str, image: &EnclaveImage<'_>) -> TokenStream {
    let name = format_ident!("{prefix}_REQUIRED_BINDINGS");
    let len = image.required_bindings.len();
    let values = image.required_bindings.values().map(|value| {
        let id = value.id().as_str();
        let kind = binding_kind(value.kind());
        quote!(RequiredBindingImage::new(BindingSlotId::new(#id), #kind))
    });
    quote!(static #name: [RequiredBindingImage; #len] = [#(#values),*];)
}

/// Renders a static level-reaction slice.
fn render_level_reactions(name: &str, values: &[LevelReactionImage]) -> TokenStream {
    let name = format_ident!("{name}");
    let len = values.len();
    let values = values.iter().map(|value| {
        let level = value.level();
        let reaction = value.reaction().as_u32();
        quote!(LevelReactionImage::new(#level, ReactionIndex::new(#reaction)))
    });
    quote!(static #name: [LevelReactionImage; #len] = [#(#values),*];)
}

/// Renders a static timer-startup slice.
fn render_timer_startups(name: &str, values: &[TimerStartupImage]) -> TokenStream {
    let name = format_ident!("{name}");
    let len = values.len();
    let values = values.iter().map(|value| {
        let action = value.action().as_u32();
        let delay = value.logical_delay_nanos();
        quote!(TimerStartupImage::new(ActionIndex::new(#action), #delay))
    });
    quote!(static #name: [TimerStartupImage; #len] = [#(#values),*];)
}

/// Renders a static lifecycle-reaction slice.
fn render_lifecycle_reactions(name: &str, values: &[LifecycleReactionImage]) -> TokenStream {
    let name = format_ident!("{name}");
    let len = values.len();
    let values = values.iter().map(|value| {
        let level = value.reaction().level();
        let reaction = value.reaction().reaction().as_u32();
        let action = value.action().as_u32();
        quote!(LifecycleReactionImage::new(
            LevelReactionImage::new(#level, ReactionIndex::new(#reaction)),
            ActionIndex::new(#action),
        ))
    });
    quote!(static #name: [LifecycleReactionImage; #len] = [#(#values),*];)
}

/// Renders a static slice of one dense key type.
fn render_indices(name: &str, ty: syn::Ident, values: impl Iterator<Item = u32>) -> TokenStream {
    let name = format_ident!("{name}");
    let values = values.collect::<Vec<_>>();
    let len = values.len();
    quote!(static #name: [#ty; #len] = [#(#ty::new(#values)),*];)
}

/// Renders packed-slice coordinates as a generated Rust constructor.
fn slice_range<T>(value: SliceRange<T>) -> TokenStream {
    let start = proc_macro2::Literal::u32_unsuffixed(value.start());
    let len = proc_macro2::Literal::u32_unsuffixed(value.len());
    quote!(SliceRange::new(#start, #len))
}

/// Renders an owner-allocated dense-key span as a generated Rust constructor.
fn index_span<K: tinymap::Key>(value: IndexSpan<K>) -> TokenStream {
    let start = proc_macro2::Literal::usize_unsuffixed(value.start());
    let len = proc_macro2::Literal::usize_unsuffixed(value.len());
    quote!(IndexSpan::new(#start, #len))
}

/// Renders an optional dense key constructor.
fn optional_index(ty: syn::Ident, value: Option<u32>) -> TokenStream {
    value.map_or_else(|| quote!(None), |value| quote!(Some(#ty::new(#value))))
}

/// Renders optional bank metadata.
fn optional_bank(value: Option<BankInfoImage>) -> TokenStream {
    value.map_or_else(
        || quote!(None),
        |value| {
            let index = value.index();
            let total = value.total();
            quote!(Some(BankInfoImage::new(#index, #total)))
        },
    )
}

/// Renders action timing metadata.
fn action_timing(value: &ActionTiming) -> TokenStream {
    match value {
        ActionTiming::Standard {
            domain,
            min_delay_nanos,
        } => {
            let domain = timing_domain(*domain);
            quote!(ActionTiming::Standard {
                domain: #domain,
                min_delay_nanos: #min_delay_nanos,
            })
        }
        ActionTiming::Timer { period_nanos } => {
            let period_nanos = period_nanos
                .map_or_else(|| quote!(None), |period_nanos| quote!(Some(#period_nanos)));
            quote!(ActionTiming::Timer {
                period_nanos: #period_nanos,
            })
        }
        ActionTiming::Shutdown => quote!(ActionTiming::Shutdown),
    }
}

/// Renders static runtime storage bounds.
fn storage_bounds(value: &StorageBounds) -> TokenStream {
    let state_slots = proc_macro2::Literal::u32_unsuffixed(value.state_slots());
    let action_slots = proc_macro2::Literal::u32_unsuffixed(value.action_slots());
    let event_capacity = proc_macro2::Literal::u32_unsuffixed(value.event_capacity());
    let payload_bytes = proc_macro2::Literal::u64_unsuffixed(value.payload_bytes());
    let state_bytes = proc_macro2::Literal::u64_unsuffixed(value.state_bytes());
    let scratch_bytes = proc_macro2::Literal::u64_unsuffixed(value.scratch_bytes());
    quote!(StorageBounds::new(
        #state_slots,
        #action_slots,
        #event_capacity,
        #payload_bytes,
        #state_bytes,
        #scratch_bytes,
    ))
}

/// Renders a timing-domain variant.
fn timing_domain(value: TimingDomain) -> TokenStream {
    match value {
        TimingDomain::Logical => quote!(TimingDomain::Logical),
        TimingDomain::Physical => quote!(TimingDomain::Physical),
    }
}

/// Renders a route-direction variant.
fn route_direction(value: RouteDirection) -> TokenStream {
    match value {
        RouteDirection::Inbound => quote!(RouteDirection::Inbound),
        RouteDirection::Outbound => quote!(RouteDirection::Outbound),
    }
}

/// Renders a binding-kind variant.
fn binding_kind(value: BindingKind) -> TokenStream {
    match value {
        BindingKind::StateInitializer => quote!(BindingKind::StateInitializer),
        BindingKind::Reaction => quote!(BindingKind::Reaction),
        BindingKind::Port => quote!(BindingKind::Port),
        BindingKind::Action => quote!(BindingKind::Action),
    }
}

/// Renders a mode-transition variant.
fn transition(value: TransitionKind) -> TokenStream {
    match value {
        TransitionKind::Reset => quote!(boomerang_runtime::TransitionKind::Reset),
        TransitionKind::History => quote!(boomerang_runtime::TransitionKind::History),
    }
}
