use boomerang::component_payload as emit;
include!("../declaration.rs");
#[cfg(feature = "source-input")]
declare_controller!(emit, source = "plant",);
#[cfg(not(feature = "source-input"))]
declare_controller!(emit,);
static INITIALIZED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static ROUTED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static SAMPLES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static REACTIONS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
#[cfg(any(feature = "fingerprint-mismatch", feature = "abi-mismatch"))]
mod incorrect_exports {
    pub use crate::controller::__boomerang::payload::*;
    pub const fn binding_manifest() -> boomerang_runtime::binding::BindingManifest {
        let actual = crate::controller::__boomerang::payload::binding_manifest();
        let fingerprint = if cfg!(feature = "fingerprint-mismatch") {
            boomerang_runtime::binding::DescriptorFingerprint::new([0; 32])
        } else {
            actual.descriptor_fingerprint()
        };
        let abi = if cfg!(feature = "abi-mismatch") {
            999
        } else {
            actual.macro_abi()
        };
        boomerang_runtime::binding::BindingManifest::new(fingerprint, abi)
    }
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
    assert_eq!(REACTIONS.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert_eq!(ROUTED.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(SAMPLES.load(std::sync::atomic::Ordering::SeqCst), 2);
    println!("two native instances executed with connected routing and initialized input");
    Ok(())
}
