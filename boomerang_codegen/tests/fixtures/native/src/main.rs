use boomerang::component_payload as emit;
include!("../declaration.rs");
declare_controller!(emit);
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
mod generated {
    include!(concat!(env!("OUT_DIR"), "/boomerang.rs"));
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    boomerang_runtime::execute_owned_federate_slice_with_observations(
        generated::FEDERATE,
        &generated::FEDERATE_VIEW,
        generated::generated_bindings(),
        boomerang_runtime::Config::default().with_fast_forward(true),
        &[],
    )?;
    assert_eq!(REACTIONS.load(std::sync::atomic::Ordering::SeqCst), 2);
    println!("two native instances executed");
    Ok(())
}
