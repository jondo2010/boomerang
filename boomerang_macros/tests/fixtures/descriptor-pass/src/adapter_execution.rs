//! Execute payload exports with real references through a dynamic startup context.

use boomerang::runtime::*;
use std::{
    ptr::NonNull,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

fn port<T: ReactorData>(value: T) -> Box<dyn BasePort> {
    let mut port = Port::<T>::new("fixture", PortKey::from(0));
    *port = Some(value);
    Box::new(port)
}

fn invoke(
    ctx: &mut Context,
    state: &mut super::shaped::ShapedState,
    inputs: &mut [Box<dyn BasePort>],
    outputs: &mut [Box<dyn BasePort>],
) -> Result<(), ReactionBindingError> {
    let mut inputs: Vec<_> = inputs
        .iter_mut()
        .map(|p| NonNull::from(p.as_mut()))
        .collect();
    let mut outputs: Vec<_> = outputs
        .iter_mut()
        .map(|p| NonNull::from(p.as_mut()))
        .collect();
    let mut startup = Action::<()>::new("startup", ActionKey::from(0), None, true);
    let mut actions = vec![NonNull::from(&mut startup as &mut dyn BaseAction)];
    let refs = ReactionRefs {
        ports: Refs::new(&mut inputs),
        ports_mut: RefsMut::new(&mut outputs),
        actions: RefsMut::new(&mut actions),
    };
    super::shaped::__boomerang::payload::reaction_Shaped_2fshaped(ctx, state, refs, None)
}

fn exercise(ctx: &mut Context) {
    let mut inputs = vec![
        port(11u16),
        port(12u16),
        port(21u64),
        port(22u64),
        port(31u32),
    ];
    let mut outputs = vec![
        port(0u32),
        port(0u32),
        port(0u32),
        port(0u8),
        port(0u8),
        port(0u8),
        port(0u64),
    ];
    let mut state = super::shaped::__boomerang::payload::state_Shaped();
    invoke(ctx, &mut state, &mut inputs, &mut outputs).unwrap();
    assert_eq!(state.calls, 1);
    for (index, port) in outputs[..3].iter().enumerate() {
        assert_eq!(
            **InputRef::<u32>::try_from(DynPortRef(port.as_ref()))
                .as_ref()
                .unwrap(),
            Some(41 + index as u32)
        );
    }
    for (index, port) in outputs[3..6].iter().enumerate() {
        assert_eq!(
            **InputRef::<u8>::try_from(DynPortRef(port.as_ref()))
                .as_ref()
                .unwrap(),
            Some(51 + index as u8)
        );
    }
    assert_eq!(
        *InputRef::<u64>::try_from(DynPortRef(outputs[6].as_ref())).unwrap(),
        Some(61)
    );

    inputs[3] = port(22u32);
    assert!(matches!(
        invoke(ctx, &mut state, &mut inputs, &mut outputs),
        Err(ReactionRefsError::TypeMismatch { .. })
    ));
    inputs[3] = port(22u64);
    let last = inputs.split_off(3);
    assert!(matches!(
        invoke(ctx, &mut state, &mut inputs, &mut outputs),
        Err(ReactionRefsError::Missing { .. })
    ));
    inputs.extend(last);
    inputs.push(port(99u32));
    assert!(matches!(
        invoke(ctx, &mut state, &mut inputs, &mut outputs),
        Err(ReactionRefsError::Remaining { remaining: 1 })
    ));
    inputs.pop();
    outputs[5] = port(0u32);
    assert!(matches!(
        invoke(ctx, &mut state, &mut inputs, &mut outputs),
        Err(ReactionRefsError::TypeMismatch { .. })
    ));
    assert_eq!(
        state.calls, 1,
        "invalid partitions never invoke the typed reaction"
    );

    let mut lifetime_state = super::lifetime_collision::__boomerang::payload::state_Lifetime();
    let mut ports = Vec::new();
    let mut ports_mut = Vec::new();
    let mut startup = Action::<()>::new("startup", ActionKey::from(0), None, true);
    let mut actions = vec![NonNull::from(&mut startup as &mut dyn BaseAction)];
    super::lifetime_collision::__boomerang::payload::reaction_Lifetime_2ftick(
        ctx,
        &mut lifetime_state,
        ReactionRefs {
            ports: Refs::new(&mut ports),
            ports_mut: RefsMut::new(&mut ports_mut),
            actions: RefsMut::new(&mut actions),
        },
        None,
    )
    .unwrap();
    assert_eq!(lifetime_state.marker, "invoked");
}

#[test]
fn adapters_preserve_reference_order_and_reject_invalid_partitions() {
    let ran = Arc::new(AtomicBool::new(false));
    let observed = ran.clone();
    let mut assembly = boomerang_builder::Assembly::new();
    let mut reactor = assembly.add_reactor("adapter-harness", None, None, (), false);
    reactor
        .add_reaction(Some("test"))
        .with_startup_trigger()
        .with_reaction_fn(move |ctx, _, (_startup,)| {
            exercise(ctx);
            observed.store(true, Ordering::SeqCst);
        })
        .finish()
        .unwrap();
    reactor.finish().unwrap();
    let config = Config::default().with_fast_forward(true);
    let runtime = assembly.into_runtime_assembly(&config).unwrap();
    let (key, enclave) = runtime.into_enclaves().next().unwrap();
    let mut scheduler = Scheduler::new(key, enclave, config, None);
    scheduler.try_event_loop().unwrap();
    assert!(ran.load(Ordering::SeqCst));
}
