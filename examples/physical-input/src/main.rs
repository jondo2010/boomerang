//! Checked-in execution policy for one compiled sensor Federate.

use boomerang_runtime::{
    clock::{ManualClock, PhysicalClockDomainId},
    image::{ActionTiming, EnclaveIndex, TimingDomain},
    input::{CompiledPhysicalAction, InputConfig, InputSource},
};

/// Generated image tables and typed bindings for the sensor deployment.
// The shared generator retains fallback fields across binding configurations.
#[allow(clippy::needless_update)]
mod generated {
    include!(concat!(env!("OUT_DIR"), "/boomerang.rs"));
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let domain = PhysicalClockDomainId(7);
    let clock = ManualClock::new(domain)?;
    // This topology gives its only physical action to the sensor's plant source.
    let enclave_index =
        EnclaveIndex::new(u32::try_from(generated::FEDERATE_VIEW.enclaves().start())?);
    let enclave = generated::FEDERATE_VIEW
        .enclave_views()
        .next()
        .ok_or("sensor enclave is absent from the compiled image")?;
    let actions = enclave
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
        .map(|(action, _)| CompiledPhysicalAction::new(enclave_index, action))
        .collect();
    let inputs = InputConfig {
        max_batch_values: 1,
        max_staged_batches: 1,
        // Source table order matches the sensor's source key in build.rs.
        sources: [InputSource {
            id: "plant".into(),
            required: true,
            actions,
        }]
        .into_iter()
        .collect(),
    };
    let bindings = generated::generated_bindings()
        .with_physical_inputs(inputs)
        .with_manual_clock(domain, clock);
    boomerang_runtime::execute_owned_federate_slice(
        generated::FEDERATE,
        &generated::FEDERATE_VIEW,
        bindings,
        boomerang_runtime::Config::default().with_fast_forward(false),
    )?;
    Ok(())
}
