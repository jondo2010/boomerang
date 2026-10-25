use boomerang::component_payload as emit;
include!("../declaration.rs");
#[path = "../nested_payload.rs"]
mod nested_payload;
static NESTED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
#[cfg(feature = "fingerprint-mismatch")]
declare_controller!(emit, shutdown,);
#[cfg(all(not(feature = "fingerprint-mismatch"), feature = "source-input"))]
declare_controller!(emit, startup, source = "plant",);
#[cfg(all(not(feature = "fingerprint-mismatch"), not(feature = "source-input")))]
declare_controller!(emit, startup,);
static INITIALIZED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static ROUTED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static SAMPLES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static REACTIONS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
/// Public export aliases retain payload-owned reaction adapters.
mod export_aliases {
    pub use crate::controller::__boomerang::payload::*;
}
/// Compiler output retains fallback fields across initialization configurations.
#[allow(clippy::needless_update)]
mod generated {
    include!(concat!(env!("OUT_DIR"), "/boomerang.rs"));
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bindings = generated::generated_bindings();
    #[cfg(feature = "source-input")]
    let bindings = {
        use boomerang_runtime::{
            image::{ActionTiming, EnclaveIndex, TimingDomain},
            input::*,
        };
        let sources = generated::FEDERATE_VIEW
            .enclave_views()
            .enumerate()
            .map(|(offset, enclave)| {
                let index = EnclaveIndex::new(
                    (generated::FEDERATE_VIEW.enclaves().start() + offset) as u32,
                );
                InputSource {
                    id: format!("source-{offset}"),
                    required: true,
                    actions: enclave
                        .actions()
                        .iter()
                        .filter(|(_, action)| {
                            matches!(
                                action.timing(),
                                ActionTiming::Standard {
                                    domain: TimingDomain::Physical,
                                    ..
                                }
                            )
                        })
                        .map(|(action, _)| CompiledPhysicalAction::new(index, action))
                        .collect(),
                }
            })
            .collect();
        bindings.with_physical_inputs(InputConfig {
            max_batch_values: 2,
            max_staged_batches: 1,
            sources,
        })
    };
    boomerang_runtime::execute_owned_federate_slice_with_observations(
        generated::FEDERATE,
        &generated::FEDERATE_VIEW,
        bindings,
        boomerang_runtime::Config::default()
            .with_timeout(boomerang_runtime::Duration::milliseconds(100)),
        &[],
    )?;
    assert_eq!(NESTED.load(std::sync::atomic::Ordering::SeqCst), 3);
    assert_eq!(REACTIONS.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert_eq!(ROUTED.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(SAMPLES.load(std::sync::atomic::Ordering::SeqCst), 2);
    println!("two native instances executed with connected routing and initialized input");
    Ok(())
}
