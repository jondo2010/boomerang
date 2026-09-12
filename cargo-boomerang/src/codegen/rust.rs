//! Deterministic Rust source rendering for generated compiled launchers.
use std::collections::BTreeMap;

use anyhow::{anyhow, bail, Context, Result};
use boomerang_builder::{
    compiler::{FederateSlice, OwnedEnclaveImage, RequiredBinding},
    ComponentDescriptor, DescriptorLifecycle, DescriptorRelationshipKind,
    DescriptorRelationshipTarget, ReactionSlot, ReactionSlotId, ReactorSlotId,
};
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

use crate::{manifest::ExecutionPolicy, DriverOutput};

/// Validates and deterministically formats one complete generated Rust file.
fn format_rust(tokens: TokenStream) -> Result<String> {
    let file = syn::parse2(tokens).context("generated Rust syntax is invalid")?;
    Ok(prettyplease::unparse(&file))
}

/// Renders one complete static launcher source file from validated compiler output.
pub(super) fn render_launcher(
    driver: &DriverOutput,
    slice: &FederateSlice<'_>,
    aliases: &BTreeMap<String, String>,
    execution: &ExecutionPolicy,
    distributed: bool,
) -> Result<String> {
    let enclaves = slice.enclaves();
    let mut route_bindings = BTreeMap::new();
    let compatibility_checks = render_compatibility_checks(driver, aliases)?;
    let enclave_images = enclaves
        .iter()
        .enumerate()
        .map(|(index, enclave)| render_enclave(index, enclave, aliases, &mut route_bindings))
        .collect::<Result<Vec<_>>>()?;
    let deployment = render_deployment(slice, !distributed);
    let bindings = render_bindings(
        driver,
        enclaves,
        slice.enclave_range().start(),
        aliases,
        route_bindings,
    )?;
    let timeout = execution.logical_horizon.map_or_else(
        || quote!(None),
        |nanos| {
            let nanos = proc_macro2::Literal::u64_unsuffixed(nanos);
            quote!(Some(boomerang_runtime::Duration::nanoseconds_i128(
                    #nanos,
            )))
        },
    );
    let main = if distributed {
        quote! {
            fn main() -> Result<(), Box<dyn std::error::Error>> {
                init_tracing();
                drop(generated_bindings());
                Err(format!(
                    "distributed generated launcher execution requires backend injection for {:?}",
                    FEDERATE_IMAGE.id(),
                )
                .into())
            }
        }
    } else {
        let fast_forward = execution.fast_forward;
        let keep_alive = execution.keep_alive;
        quote! {
            fn main() -> Result<(), Box<dyn std::error::Error>> {
                init_tracing();
                let bindings = generated_bindings();
                let execution = execute_owned_federate(
                    &DEPLOYMENT,
                    FEDERATE,
                    bindings,
                    Config {
                        fast_forward: #fast_forward,
                        timeout: #timeout,
                        keep_alive: #keep_alive,
                        // Legacy public-API compatibility placeholder.
                        physical_event_q_size: 1024,
                    },
                )?;
                write_execution_summary(&execution)?;
                Ok(())
            }
        }
    };
    let tokens = quote! {
        use boomerang_runtime::{
            execute_owned_federate, Config, EnclaveBindings, FederateBindings, ReactorData,
        };
        use boomerang_runtime::image::*;
        use tinymap::{IndexSpan, SliceRange, TinyMapView};

        fn generated_state<'a, T: ReactorData>(
            state: &'a mut dyn ReactorData,
            _initializer: fn() -> T,
        ) -> &'a mut T {
            state
                .downcast_mut::<T>()
                .expect("generated state initializer and reaction must agree")
        }

        const EXECUTION_SUMMARY_ENV: &str = "BOOMERANG_EXECUTION_SUMMARY_V1";

        /// Installs launcher tracing unless this process already has a subscriber.
        fn init_tracing() {
            let filter = tracing_subscriber::EnvFilter::builder()
                .with_default_directive(
                    tracing_subscriber::filter::LevelFilter::OFF.into(),
                )
                .from_env_lossy();
            let ansi = std::io::IsTerminal::is_terminal(&std::io::stderr())
                && std::env::var_os("NO_COLOR").map_or(true, |value| value.is_empty());
            let _ = tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_ansi(ansi)
                .with_writer(std::io::stderr)
                .try_init();
        }

        /// Writes the optional version-1 execution summary for the supervising host.
        fn write_execution_summary(
            execution: &boomerang_runtime::FederateExecution,
        ) -> std::io::Result<()> {
            let Some(path) = std::env::var_os(EXECUTION_SUMMARY_ENV) else {
                return Ok(());
            };
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)?;
            let stats = execution.stats();
            use std::io::Write as _;
            writeln!(
                file,
                concat!(
                    "{{\"schema\":1,\"stats\":{{",
                    "\"processed_tags\":\"{}\",",
                    "\"processed_reactions\":\"{}\",",
                    "\"processed_events\":\"{}\",",
                    "\"set_ports\":\"{}\",",
                    "\"scheduled_actions\":\"{}\"}},",
                    "\"final_tag\":{{\"offset_nanos\":\"{}\",",
                    "\"microstep\":\"{}\"}}}}",
                ),
                stats.processed_tags(),
                stats.processed_reactions(),
                stats.processed_events(),
                stats.set_ports(),
                stats.scheduled_actions(),
                execution.final_tag().offset().whole_nanoseconds(),
                execution.final_tag().microstep(),
            )
        }

        #compatibility_checks
        #(#enclave_images)*
        #deployment
        #bindings
        #main
    };
    let formatted = format_rust(tokens)?;
    Ok(format!(
        "// @generated by cargo-boomerang; do not edit.\n{formatted}"
    ))
}

/// Emits independent const checks for every selected descriptor/payload pair.
fn render_compatibility_checks(
    driver: &DriverOutput,
    aliases: &BTreeMap<String, String>,
) -> Result<TokenStream> {
    let mut checks = TokenStream::new();
    for binding in driver.bindings() {
        let implementation = binding.implementation().as_str();
        let Some(alias) = aliases.get(implementation) else {
            continue;
        };
        let descriptor = binding.descriptor();
        let bytes = descriptor
            .descriptor_fingerprint_input()
            .fingerprint()
            .to_bytes();
        let bytes = bytes.iter();
        let alias = rust_ident(alias, "crate alias")?;
        let abi = descriptor.macro_abi();
        checks.extend(quote! {
            const _: () = boomerang_runtime::binding::assert_descriptor_fingerprint(
                boomerang_runtime::binding::DescriptorFingerprint::new([#(#bytes),*]),
                #alias::__boomerang::BINDING_MANIFEST.descriptor_fingerprint(),
            );
            const _: () = assert!(
                #abi == #alias::__boomerang::BINDING_MANIFEST.macro_abi(),
                "macro ABI mismatch",
            );
        });
    }
    Ok(checks)
}

/// Emits every immutable scheduler table for one compiled Enclave.
fn render_enclave(
    index: usize,
    owned: &OwnedEnclaveImage,
    aliases: &BTreeMap<String, String>,
    route_bindings: &mut RouteBindings,
) -> Result<TokenStream> {
    owned.with_image(|image| {
        let tokens = render_enclave_image(index, &image);
        collect_route_bindings(route_bindings, owned, &image, aliases)?;
        Ok(tokens)
    })
}

/// Renders every immutable scheduler table for one borrowed enclave image.
fn render_enclave_image(index: usize, image: &EnclaveImage<'_>) -> TokenStream {
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
        static #image_name: EnclaveImage<'static> = EnclaveImage {
            enclave_id: EnclaveId::new(#enclave_id),
            reactors: TinyMapView::new(&#reactors),
            actions: TinyMapView::new(&#actions),
            ports: TinyMapView::new(&#ports),
            reactions: TinyMapView::new(&#reactions),
            modes: TinyMapView::new(&#modes),
            scopes: TinyMapView::new(&#scopes),
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
            routes: TinyMapView::new(&#routes),
            required_bindings: TinyMapView::new(&#required_bindings),
            storage_bounds: #bounds,
        };
    });
    tokens
}

/// Emits the deployment root tables around the selected Federate's Enclave images.
fn render_deployment(slice: &FederateSlice<'_>, include_local_deployment: bool) -> TokenStream {
    let enclave_count = slice.enclaves().len();
    let enclave_len = proc_macro2::Literal::usize_unsuffixed(enclave_count);
    let enclave_images = (0..enclave_count).map(|index| format_ident!("E{index}_IMAGE"));
    let federate = proc_macro2::Literal::u32_unsuffixed(slice.federate().as_u32());
    let id = slice.id().as_str();
    let target = slice.target().as_str();
    let runtime = slice.runtime().as_str();
    let enclave_range = index_span(slice.enclave_range());
    let deployment = include_local_deployment.then(|| {
        quote!(
            static DEPLOYMENT: CompiledDeploymentImage<'static> = CompiledDeploymentImage {
                federation: GlobalFederationImage::new(&FEDERATION_MEMBERS, &[]),
                federates: TinyMapView::new(&FEDERATES),
                enclaves: TinyMapView::new(&ENCLAVES),
                coordination: CoordinationProjection::Local,
            };
        )
    });
    quote! {
        static ENCLAVES: [EnclaveImage<'static>; #enclave_len] = [#(#enclave_images),*];
        static FEDERATE: FederateIndex = FederateIndex::new(#federate);
        static FEDERATE_IMAGE: FederateImage<'static> = FederateImage::new(
            FederateId::new(#id),
            TargetId::new(#target),
            RuntimeBackendId::new(#runtime),
            #enclave_range,
        );
        static FEDERATES: [FederateImage; 1] = [FEDERATE_IMAGE];
        static FEDERATION_MEMBERS: [FederateIndex; 1] = [FEDERATE];
        #deployment
    }
}

/// Emits direct typed binding construction for every Enclave.
fn render_bindings(
    driver: &DriverOutput,
    enclaves: &[OwnedEnclaveImage],
    enclave_start: usize,
    aliases: &BTreeMap<String, String>,
    route_bindings: RouteBindings,
) -> Result<TokenStream> {
    let mut enclave_bindings = Vec::with_capacity(enclaves.len());
    for (enclave_offset, enclave) in enclaves.iter().enumerate() {
        let enclave_index = u32::try_from(
            enclave_start
                .checked_add(enclave_offset)
                .ok_or_else(|| anyhow!("compiled Enclave index exceeds the image domain"))?,
        )?;
        let enclave_index = proc_macro2::Literal::u32_unsuffixed(enclave_index);
        let mut bindings = TokenStream::new();
        for (slot, binding) in enclave.required_bindings().iter().enumerate() {
            bindings.extend(render_binding(driver, aliases, slot, binding)?);
        }
        enclave_bindings.push(quote!(
            .bind_enclave(
                EnclaveIndex::new(#enclave_index),
                EnclaveBindings::new() #bindings,
            )
        ));
    }
    let route_bindings = render_route_bindings(route_bindings);
    Ok(quote!(
        fn generated_bindings() -> FederateBindings<'static> {
            FederateBindings::new()
                #(#enclave_bindings)*
                #route_bindings
        }
    ))
}

/// Accumulated generated payload types for each local scheduler boundary.
type RouteBindings = BTreeMap<String, (Option<TokenStream>, Option<TokenStream>)>;

/// Validates one externally derived Rust identifier before token interpolation.
fn rust_ident(value: &str, role: &str) -> Result<syn::Ident> {
    syn::parse_str(value).with_context(|| format!("invalid generated {role} identifier '{value}'"))
}

/// Renders a generated payload binding path from validated identifier segments.
fn binding_path(alias: &str, symbol: &str) -> Result<TokenStream> {
    let alias = rust_ident(alias, "crate alias")?;
    let symbol = rust_ident(symbol, "binding symbol")?;
    Ok(quote!(#alias::__boomerang::#symbol))
}

/// Collects paired route payloads while each enclave image is borrowed.
fn collect_route_bindings(
    routes: &mut RouteBindings,
    enclave: &OwnedEnclaveImage,
    image: &EnclaveImage<'_>,
    aliases: &BTreeMap<String, String>,
) -> Result<()> {
    for route in image.routes.values().copied() {
        let boundary = route.boundary().as_str().to_owned();
        let port = image.ports[route.local_port()];
        let binding = enclave
            .required_bindings()
            .iter()
            .nth(usize::try_from(port.binding().as_u32())?)
            .expect("compiled port binding index is validated");
        let RequiredBinding::Port { implementation, .. } = binding else {
            bail!("route '{boundary}' local port does not have a port binding");
        };
        let alias = aliases.get(implementation.as_str()).ok_or_else(|| {
            anyhow!("missing generated alias for implementation '{implementation}'")
        })?;
        let payload = binding_path(alias, &binding.symbol())?;
        let pair = routes.entry(boundary.clone()).or_default();
        let slot = match route.direction() {
            RouteDirection::Outbound => &mut pair.0,
            RouteDirection::Inbound => &mut pair.1,
        };
        if slot.replace(payload).is_some() {
            bail!(
                "route '{boundary}' has duplicate {:?} halves",
                route.direction()
            );
        }
    }
    Ok(())
}

/// Emits one typed route binding for each paired local scheduler boundary.
fn render_route_bindings(routes: RouteBindings) -> TokenStream {
    let mut tokens = TokenStream::new();
    for (boundary, (source_payload, destination_payload)) in routes {
        let (Some(source_payload), Some(destination_payload)) =
            (source_payload, destination_payload)
        else {
            continue;
        };
        tokens.extend(quote!(
            .bind_route(
                BoundaryId::new(#boundary),
                #source_payload,
                #destination_payload,
            )
        ));
    }
    tokens
}

/// Emits one state, reaction, port, or action binding chain entry.
fn render_binding(
    driver: &DriverOutput,
    aliases: &BTreeMap<String, String>,
    slot: usize,
    binding: &RequiredBinding,
) -> Result<TokenStream> {
    let slot = u32::try_from(slot).context("generated binding slot exceeds u32 range")?;
    let slot = proc_macro2::Literal::u32_unsuffixed(slot);
    let (implementation, symbol) = match binding {
        RequiredBinding::State { implementation, .. }
        | RequiredBinding::Reaction { implementation, .. }
        | RequiredBinding::Port { implementation, .. }
        | RequiredBinding::Action { implementation, .. } => {
            (implementation.as_str(), binding.symbol())
        }
    };
    let alias = aliases
        .get(implementation)
        .ok_or_else(|| anyhow!("missing generated alias for implementation '{implementation}'"))?;
    let path = binding_path(alias, &symbol)?;
    match binding {
        RequiredBinding::State { .. } => Ok(quote!(
            .bind_state(BindingSlotIndex::new(#slot), #path)
        )),
        RequiredBinding::Port { .. } => Ok(quote!(
            .bind_port(BindingSlotIndex::new(#slot), #path)
        )),
        RequiredBinding::Action { .. } => Ok(quote!(
            .bind_action(BindingSlotIndex::new(#slot), #path)
        )),
        RequiredBinding::Reaction { reaction, .. } => {
            let descriptor = driver
                .bindings()
                .iter()
                .find(|binding| binding.implementation().as_str() == implementation)
                .expect("required implementation has descriptor output")
                .descriptor();
            let state = reaction_reactor(descriptor.reaction_slots(), reaction)?;
            let state_symbol = boomerang_builder::compiler::direct_binding_symbol(
                BindingKind::StateInitializer,
                state.path(),
            );
            render_reaction_binding(descriptor, reaction, slot, alias, &state_symbol, &symbol)
        }
    }
}

/// Resolves a reaction slot to the exact descriptor reactor that owns it.
fn reaction_reactor<'a>(
    slots: &'a [ReactionSlot],
    reaction: &ReactionSlotId,
) -> Result<&'a ReactorSlotId> {
    slots
        .iter()
        .find(|slot| slot.id == *reaction)
        .map(|slot| &slot.reactor)
        .ok_or_else(|| anyhow!("reaction '{reaction}' has no descriptor reactor"))
}

/// Emits the erased-to-typed adapter for one generated payload reaction function.
fn render_reaction_binding(
    descriptor: &ComponentDescriptor,
    reaction: &ReactionSlotId,
    slot: proc_macro2::Literal,
    alias: &str,
    state_symbol: &str,
    reaction_symbol: &str,
) -> Result<TokenStream> {
    let relations = descriptor
        .relationships()
        .iter()
        .filter(|relation| relation.reaction == *reaction)
        .collect::<Vec<_>>();
    let mut immutable_ports = Vec::new();
    let mut mutable_ports = Vec::new();
    let mut actions = Vec::new();
    let mut tuple = Vec::new();
    let has_mode_effect = relations.iter().any(|relation| {
        relation.kind == DescriptorRelationshipKind::Effect
            && matches!(&relation.target, DescriptorRelationshipTarget::Mode(_))
            && relation.mode_transition.is_some()
    });
    for kind in [
        DescriptorRelationshipKind::Trigger,
        DescriptorRelationshipKind::Use,
        DescriptorRelationshipKind::Effect,
    ] {
        let mut category = relations
            .iter()
            .copied()
            .filter(|relation| relation.kind == kind)
            .collect::<Vec<_>>();
        category.sort_by_key(|relation| relation.declaration_position);
        for relation in category {
            let name = match (&relation.target, kind) {
                (
                    DescriptorRelationshipTarget::Port(_),
                    DescriptorRelationshipKind::Trigger | DescriptorRelationshipKind::Use,
                ) => {
                    let name = next_name("port", &mut immutable_ports);
                    quote!(#name)
                }
                (DescriptorRelationshipTarget::Port(_), DescriptorRelationshipKind::Effect) => {
                    let name = next_name("port_mut", &mut mutable_ports);
                    quote!(#name)
                }
                (DescriptorRelationshipTarget::Action(_), _) => {
                    let name = next_name("action", &mut actions);
                    quote!(#name)
                }
                (
                    DescriptorRelationshipTarget::Lifecycle(
                        DescriptorLifecycle::Startup | DescriptorLifecycle::Shutdown,
                    ),
                    DescriptorRelationshipKind::Trigger,
                ) => {
                    let name = next_name("action", &mut actions);
                    quote!(#name)
                }
                (
                    DescriptorRelationshipTarget::Lifecycle(DescriptorLifecycle::Reset),
                    DescriptorRelationshipKind::Trigger,
                ) => continue,
                (DescriptorRelationshipTarget::Mode(_), DescriptorRelationshipKind::Effect)
                    if relation.mode_transition.is_some() =>
                {
                    quote!(mode_effect.expect("compiled mode effect is required"))
                }
                (_, DescriptorRelationshipKind::Mode | DescriptorRelationshipKind::Scope) => {
                    continue
                }
                (target, _) => bail!("unsupported generated reaction relationship {target:?}"),
            };
            tuple.push(name);
        }
    }
    let mode_effect = if has_mode_effect {
        format_ident!("mode_effect")
    } else {
        format_ident!("_mode_effect")
    };
    let state = binding_path(alias, state_symbol)?;
    let reaction = binding_path(alias, reaction_symbol)?;
    let immutable_partition = render_partition(quote!(refs.ports.partition()?), &immutable_ports);
    let mutable_partition =
        render_partition(quote!(refs.ports_mut.partition_mut()?), &mutable_ports);
    let action_partition = render_partition(quote!(refs.actions.partition_mut()?), &actions);
    Ok(quote!(
        .bind_reaction(
            BindingSlotIndex::new(#slot),
            |ctx, state, refs, #mode_effect| {
                let state = generated_state(state, #state);
                #immutable_partition
                #mutable_partition
                #action_partition
                #reaction(ctx, state, (#(#tuple,)*));
                Ok(())
            },
        )
    ))
}

/// Allocates the next generated local name within one reference partition.
fn next_name(prefix: &str, values: &mut Vec<syn::Ident>) -> syn::Ident {
    let name = format_ident!("{prefix}_{}", values.len());
    values.push(name.clone());
    name
}

/// Emits a tuple destructure whose inferred element types drive runtime reference extraction.
fn render_partition(expression: TokenStream, values: &[syn::Ident]) -> TokenStream {
    if values.is_empty() {
        return TokenStream::new();
    }
    quote!(let (#(#values,)*) = #expression;)
}

/// Renders the immutable reactor table for one enclave.
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
fn action_timing(value: ActionTiming) -> TokenStream {
    match value {
        ActionTiming::Standard {
            domain,
            min_delay_nanos,
        } => {
            let domain = timing_domain(domain);
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
fn storage_bounds(value: StorageBounds) -> TokenStream {
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

#[cfg(test)]
mod tests {
    use boomerang_builder::{ReactionSlot, ReactionSlotId, ReactorSlotId};
    use quote::quote;

    use super::{format_rust, reaction_reactor};

    /// Confirms malformed structural output fails at the publication boundary.
    #[test]
    fn malformed_generated_tokens_are_rejected_before_publication() {
        let error = format_rust(quote!(fn incomplete)).unwrap_err();

        assert!(
            error.to_string().contains("generated Rust syntax"),
            "{error:#}"
        );
    }

    /// Confirms nested reactions resolve their exact descriptor state owner.
    #[test]
    fn nested_reaction_uses_its_exact_descriptor_owner() {
        let root = ReactorSlotId::new("Root").unwrap();
        let child = ReactorSlotId::new("Root/child").unwrap();
        let reaction = ReactionSlotId::new("Root/child/startup").unwrap();
        let slots = [ReactionSlot {
            id: reaction.clone(),
            reactor: child.clone(),
        }];

        assert_eq!(reaction_reactor(&slots, &reaction).unwrap(), &child);
        assert_ne!(reaction_reactor(&slots, &reaction).unwrap(), &root);
    }
}
