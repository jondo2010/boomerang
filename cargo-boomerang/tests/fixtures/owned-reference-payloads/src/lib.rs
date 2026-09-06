//! Fixture-owned payload bindings for the generated-versus-owned differential.

extern crate self as boomerang;

/// Runtime API expected by the payload facet's generated symbols.
pub use boomerang_runtime as runtime;

/// Minimal payload-facet prelude used by the included fixture sources.
pub mod prelude {
    pub use boomerang_macros::{reaction, reactor, reactor_ports, timer};
    pub use boomerang_runtime::{self as runtime, CommonContext, Duration, FromRefs, Tag};
}

/// Controller payload compiled directly from the fixture-owned source.
pub mod controller {
    include!("../../workspace/vehicle-control/src/lib.rs");
}

/// Sensor payload compiled directly from the fixture-owned source.
pub mod sensor {
    include!("../../workspace/sensor-host/src/lib.rs");
}

use boomerang::runtime::{
    image::{BindingSlotIndex, BoundaryId, EnclaveIndex},
    Context, EnclaveBindings, FederateBindings, ReactionBindingError, ReactionRefs, ReactorData,
};

fn generated_state<T: ReactorData>(state: &mut dyn ReactorData, _initializer: fn() -> T) -> &mut T {
    state
        .downcast_mut::<T>()
        .expect("fixture state initializer and reaction must agree")
}

fn controller_shutdown(
    ctx: &mut Context,
    state: &mut dyn ReactorData,
    refs: ReactionRefs<'_>,
) -> Result<(), ReactionBindingError> {
    let state = generated_state(state, controller::__boomerang::state_Controller);
    let (shutdown,) = refs.actions.partition_mut()?;
    controller::__boomerang::reaction_Controller_2f_23g0(ctx, state, (shutdown,));
    Ok(())
}

fn controller_control(
    ctx: &mut Context,
    state: &mut dyn ReactorData,
    refs: ReactionRefs<'_>,
) -> Result<(), ReactionBindingError> {
    let state = generated_state(state, controller::__boomerang::state_Controller);
    let (command,) = refs.ports_mut.partition_mut()?;
    let (startup,) = refs.actions.partition_mut()?;
    controller::__boomerang::reaction_Controller_2fcontrol(ctx, state, (startup, command));
    Ok(())
}

fn sensor_sample(
    ctx: &mut Context,
    state: &mut dyn ReactorData,
    refs: ReactionRefs<'_>,
) -> Result<(), ReactionBindingError> {
    let state = generated_state(state, sensor::__boomerang::state_Sensor);
    let (command,) = refs.ports.partition()?;
    sensor::__boomerang::reaction_Sensor_2fsample(ctx, state, (command,));
    Ok(())
}

fn controller_bindings() -> EnclaveBindings {
    EnclaveBindings::new()
        .bind_port(
            BindingSlotIndex::new(0),
            controller::__boomerang::port_Controller_2fcommand,
        )
        .bind_reaction(BindingSlotIndex::new(1), |ctx, state, refs, _| {
            controller_shutdown(ctx, state, refs)
        })
        .bind_reaction(BindingSlotIndex::new(2), |ctx, state, refs, _| {
            controller_control(ctx, state, refs)
        })
        .bind_state(
            BindingSlotIndex::new(3),
            controller::__boomerang::state_Controller,
        )
}

/// Returns the independently owned payload binding table for the production fixture.
pub fn bindings() -> FederateBindings<'static> {
    FederateBindings::new()
        .bind_enclave(EnclaveIndex::new(0), controller_bindings())
        .bind_enclave(EnclaveIndex::new(1), controller_bindings())
        .bind_enclave(
            EnclaveIndex::new(2),
            EnclaveBindings::new()
                .bind_port(
                    BindingSlotIndex::new(0),
                    sensor::__boomerang::port_Sensor_2fcommand,
                )
                .bind_reaction(BindingSlotIndex::new(1), |ctx, state, refs, _| {
                    sensor_sample(ctx, state, refs)
                })
                .bind_state(BindingSlotIndex::new(2), sensor::__boomerang::state_Sensor),
        )
        .bind_route(
            BoundaryId::new("boundary/controller%2Fcommand/sensor%2Fcommand/c0"),
            controller::__boomerang::port_Controller_2fcommand,
            sensor::__boomerang::port_Sensor_2fcommand,
        )
}
