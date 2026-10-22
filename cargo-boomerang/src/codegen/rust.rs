//! Deterministic Rust source rendering for generated compiled launchers.
use std::collections::BTreeMap;

use anyhow::{anyhow, bail, Result};
use boomerang_builder::compiler::{ComponentInstanceId, FederateSlice, RequiredBinding};
pub(crate) use boomerang_codegen::render::format_rust;
pub(super) use boomerang_codegen::render::image_fingerprint_input;
use boomerang_codegen::{
    render::{render_definitions, BindingConfiguration},
    ComponentRegistration, ManifestStyle, PayloadExports,
};
use boomerang_runtime::image::{ActionTiming, TimingDomain};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use crate::{
    codegen::LauncherCapabilities,
    manifest::{ExecutionPolicy, TelemetryBackend, TracingBackend},
    DriverOutput,
};

/// Emits only the initialization for the selected build-time backend.
pub(super) fn render_tracing_init(
    backend: TracingBackend,
    limits: Option<&crate::BoundedTracingLimits>,
) -> TokenStream {
    match backend {
        TracingBackend::Off => quote! {},
        TracingBackend::Hosted => quote! {
            let _tracing_guard = boomerang_util::launcher::init_tracing();
        },
        TracingBackend::Bounded => {
            let limits = limits.expect("analyzed bounded launchers have resolved limits");
            let level = format_ident!("{}", limits.level.to_string().to_ascii_uppercase());
            let targets = &limits.targets;
            let [records, fields, bytes, spans, span_fields, span_bytes, producers, depth] = [
                limits.records,
                limits.fields,
                limits.bytes,
                limits.spans,
                limits.span_fields,
                limits.span_bytes,
                limits.producers,
                limits.depth,
            ]
            .map(proc_macro2::Literal::u32_unsuffixed);
            quote! {
                let _tracing_guard = boomerang_util::launcher::init_bounded_tracing(
                    boomerang_util::launcher::BoundedTracingConfig {
                        level: boomerang_util::launcher::LevelFilter::#level,
                        targets: &[#(#targets),*],
                        records: #records, fields: #fields, bytes: #bytes,
                        spans: #spans, span_fields: #span_fields, span_bytes: #span_bytes,
                        producers: #producers, depth: #depth,
                        ..Default::default()
                    }
                )?;
            }
        }
    }
}

/// Create the manual clock and bind it to the Federate.
fn render_physical_clock(clock: &crate::manifest::ExternalClock) -> TokenStream {
    let domain = clock.domain;
    quote! {
        let clock_domain = boomerang_runtime::clock::PhysicalClockDomainId(#domain);
        let manual_clock = boomerang_runtime::clock::ManualClock::new(clock_domain)?;
        let bindings = bindings.with_manual_clock(clock_domain, manual_clock);
    }
}

/// Resolves source policy once against original compiled action rows.
fn render_physical_inputs(
    inputs: &crate::manifest::PhysicalInputs,
    slice: &FederateSlice<'_>,
    driver: &DriverOutput,
) -> Result<TokenStream> {
    let max_values = inputs.max_batch_values;
    let max_batches = inputs.max_staged_batches;
    let mut sources = Vec::new();
    let mut owned = Vec::new();
    let mut policy_owners = std::collections::BTreeSet::new();
    for (id, source) in &inputs.sources {
        if !policy_owners.insert(&source.component) {
            bail!(
                "component '{}' has duplicate physical source policies",
                source.component
            )
        }
        let required = source.required;
        let mut actions = Vec::new();
        let mut owners = 0;
        for (offset, enclave) in slice.enclaves().iter().enumerate() {
            for binding in enclave.required_bindings().iter() {
                let RequiredBinding::State {
                    component,
                    reactor,
                    implementation,
                } = binding
                else {
                    continue;
                };
                if component.to_string() != source.component {
                    continue;
                }
                let descriptor = driver
                    .bindings()
                    .iter()
                    .find(|binding| binding.implementation() == implementation)
                    .expect("selected component descriptor")
                    .descriptor();
                let Some(capabilities) = descriptor
                    .reactor_slots()
                    .iter()
                    .find(|slot| &slot.id == reactor)
                    .and_then(|slot| slot.capabilities.as_ref())
                    .filter(|capabilities| capabilities.source.is_some())
                else {
                    continue;
                };
                owners += 1;
                for action in &capabilities.actions {
                    let (slot, _) = enclave.required_bindings().iter().enumerate().find(|(_, binding)| matches!(binding, RequiredBinding::Action { component: owner, action: candidate, .. } if owner == component && candidate == action)).ok_or_else(|| anyhow!("physical source '{id}' refers to missing action '{action}'"))?;
                    let action = enclave.with_image(|image| -> Result<_> {
                        let slot =
                            boomerang_runtime::image::BindingSlotIndex::new(u32::try_from(slot)?);
                        let (index, action) = image
                            .actions
                            .iter()
                            .find(|(_, action)| action.binding() == Some(slot))
                            .ok_or_else(|| {
                                anyhow!("physical source '{id}' binding is not an action")
                            })?;
                        if !matches!(
                            action.timing(),
                            ActionTiming::Standard {
                                domain: TimingDomain::Physical,
                                ..
                            }
                        ) {
                            bail!("physical source '{id}' binding is not a physical action")
                        }
                        Ok(index)
                    })?;
                    let enclave_index = u32::try_from(
                        slice
                            .enclave_range()
                            .start()
                            .checked_add(offset)
                            .ok_or_else(|| anyhow!("Enclave index overflow"))?,
                    )?;
                    if owned.contains(&(enclave_index, action)) {
                        bail!("physical source '{id}' duplicates action ownership")
                    }
                    owned.push((enclave_index, action));
                    let action = action.as_u32();
                    actions.push(quote!(boomerang_runtime::input::CompiledPhysicalAction::new(EnclaveIndex::new(#enclave_index), ActionIndex::new(#action))));
                }
            }
        }
        if owners != 1 || actions.is_empty() {
            bail!("physical source '{id}' must have one declared component owner and at least one physical action")
        }
        sources.push(quote!(boomerang_runtime::input::InputSource { id: #id.into(), required: #required, actions: vec![#(#actions),*] }));
    }
    Ok(
        quote!(let bindings = bindings.with_physical_inputs(boomerang_runtime::input::InputConfig { max_batch_values: #max_values, max_staged_batches: #max_batches, sources: [#(#sources),*].into_iter().collect() });),
    )
}

/// Renders one complete static launcher source file from validated compiler output.
pub(super) struct LauncherInstrumentation {
    pub(super) physical_clock: Option<crate::manifest::ExternalClock>,
    pub(super) physical_inputs: Option<crate::manifest::PhysicalInputs>,
    pub(super) tracing: TokenStream,
    pub(super) telemetry: TelemetryBackend,
}

pub(super) fn render_launcher(
    driver: &DriverOutput,
    slice: &FederateSlice<'_>,
    aliases: &BTreeMap<String, String>,
    execution: &ExecutionPolicy,
    instrumentation: LauncherInstrumentation,
    coordination: Option<TokenStream>,
    capabilities: LauncherCapabilities,
) -> Result<String> {
    let LauncherInstrumentation {
        tracing,
        telemetry,
        physical_clock,
        physical_inputs,
    } = instrumentation;
    let fingerprint = super::fingerprints::with_physical_clock(
        super::fingerprints::federate_image(slice, driver.bindings())?,
        physical_clock.as_ref(),
    )?;
    let fingerprint =
        super::fingerprints::with_physical_inputs(fingerprint, physical_inputs.as_ref())?;
    let clock_binding = physical_clock.as_ref().map(render_physical_clock);
    let input_binding = physical_inputs
        .as_ref()
        .map(|inputs| render_physical_inputs(inputs, slice, driver))
        .transpose()?;
    let fingerprint_bytes = fingerprint.as_bytes().iter();
    let distributed = coordination.is_some();
    let enclaves = slice.enclaves();
    let components = driver
        .bindings()
        .iter()
        .filter_map(|binding| {
            aliases.get(binding.implementation().as_str()).map(|alias| {
                // Legacy module and crate-level exports have different manifest forms.
                let path: syn::Path = syn::parse_str(alias)?;
                let style = if path.segments.len() > 1 {
                    ManifestStyle::Function
                } else {
                    ManifestStyle::Constant
                };
                Ok((
                    binding.implementation().clone(),
                    ComponentRegistration {
                        descriptor: binding.descriptor().clone(),
                        payload: PayloadExports::with_manifest(
                            format!("{alias}::__boomerang"),
                            style,
                        )?,
                    },
                ))
            })
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    let sources = physical_inputs
        .as_ref()
        .map(|inputs| {
            inputs.sources.iter().enumerate().try_fold(
                BTreeMap::new(),
                |mut sources, (index, (_, policy))| -> Result<_> {
                    // Preserve the legacy first matching source policy for each owner.
                    sources
                        .entry(ComponentInstanceId::new(policy.component.as_str())?)
                        .or_insert(u32::try_from(index)?);
                    Ok(sources)
                },
            )
        })
        .transpose()?
        .unwrap_or_default();
    let definitions = render_definitions(
        slice,
        &components,
        &BindingConfiguration {
            sources,
            manual_clock: physical_clock.is_some(),
            distributed,
        },
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
    let init_tracing = capabilities.hosted.then_some(tracing);
    let write_execution_summary = capabilities
        .hosted
        .then(|| quote!(boomerang_util::launcher::write_execution_summary(&execution)?;));
    let fast_forward = execution.fast_forward;
    let keep_alive = execution.keep_alive;
    let (telemetry_init, observation) = if telemetry == TelemetryBackend::Hosted {
        let federate_id = slice.id().as_str();
        let sources = enclaves.iter().map(|enclave| {
            let enclave_id = enclave.id().to_canonical_string();
            quote!(boomerang_telemetry::SourceIdentity {
                role: boomerang_telemetry::SourceRole::Federate,
                federate_id: Some(#federate_id), enclave_id: Some(#enclave_id),
            })
        });
        let keys = (0..enclaves.len()).map(|offset| {
            let key = slice.enclave_range().start() + offset;
            quote!(EnclaveIndex::new(#key as u32))
        });
        (
            quote! {
                let _telemetry = boomerang_util::telemetry::TelemetryExporter::from_environment(
                    FEDERATE_IMAGE_FINGERPRINT.bytes(),
                    #federate_id,
                    0,
                    [#(#sources),*],
                )?;
            },
            quote! {
                _telemetry.as_ref().map_or_else(Vec::new, |exporter| {
                    [#(#keys),*].into_iter().zip(exporter.observation_handles()).collect()
                })
            },
        )
    } else {
        (quote! {}, quote!(Vec::new()))
    };
    let config = quote!(Config {
        fast_forward: #fast_forward,
        timeout: #timeout,
        keep_alive: #keep_alive,
        physical_event_q_size: 1024,
    });
    let execute = if distributed {
        quote! {
            use boomerang_central_rti::compiled::{CentralRtiClient, RtiClientBindings, hosted};
            let timeout = std::time::Duration::from_secs(10);
            let address = std::env::var("BOOMERANG_RTI_ADDRESS")
                .map_err(|_| "BOOMERANG_RTI_ADDRESS is required for central-rti execution")?
                .parse()?;
            let view = RtiImageView::new(COORDINATION_IMAGE, COORDINATION_MEMBERS)?;
            let rti_bindings = RtiClientBindings::from_image(view, FEDERATE, COORDINATION_IDENTITY)?;
            let _execution_span = rti_bindings.execution_span().entered();
            let connection = hosted::connect(address, FEDERATE, wire_contract(), timeout)?;
            let sink = connection.sink();
            let bindings = generated_bindings(&rti_bindings, sink.clone())?;
            let execution = boomerang_runtime::execute_owned_federate_with_backend_and_observations(
                FEDERATE, &FEDERATE_VIEW, bindings, #config,
                &telemetry_observations,
                |inbound| CentralRtiClient::connect(sink, connection, rti_bindings, inbound, timeout),
            )?;
        }
    } else {
        quote! {
            let bindings = generated_bindings();
            #clock_binding
        #input_binding
            let execution = boomerang_runtime::execute_owned_federate_slice_with_observations(
                FEDERATE, &FEDERATE_VIEW, bindings, #config,
                &telemetry_observations,
            )?;
        }
    };
    let main = quote! {
        fn main() -> Result<(), Box<dyn std::error::Error>> {
            #init_tracing
            #telemetry_init
            let telemetry_observations = #observation;
            #execute
            #write_execution_summary
            Ok(())
        }
    };
    let tokens = quote! {
        #definitions
        use boomerang_runtime::Config;
        /// Identity of this executable's scheduler image and local binding requirements.
        pub const FEDERATE_IMAGE_FINGERPRINT: boomerang_federated::wire::FederateImageFingerprint =
            boomerang_federated::wire::FederateImageFingerprint::new([#(#fingerprint_bytes),*]);
        #coordination
        #main
    };
    let formatted = format_rust(tokens)?;
    Ok(format!(
        "// @generated by cargo-boomerang; do not edit.\n{formatted}"
    ))
}
