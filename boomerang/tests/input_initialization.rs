//! Live runners initialize ordinary input before logical startup.
use boomerang::prelude::*;
use std::sync::atomic::{AtomicUsize, Ordering};
static INITIALIZATIONS: AtomicUsize = AtomicUsize::new(0);
#[derive(Debug)]
struct InputState {
    started: bool,
    received: Vec<u32>,
}
impl InputState {
    fn new(init: InputInit) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        INITIALIZATIONS.fetch_add(1, Ordering::SeqCst);
        assert!(init.send.try_get_physical_time().is_ok());
        init.send
            .prepare_action(&init.sample, 42, None)?
            .try_send()?;
        Ok(Self {
            started: false,
            received: Vec::new(),
        })
    }
}
#[reactor(state = InputState, state_init = InputState::new, init_context(name = InputInit, actions = [sample]))]
fn Input(#[physical_action(min_delay = 1 msec)] sample: u32) -> impl Reactor {
    reaction! { Start(startup) { state.started = true; } }
    reaction! { Receive(sample) {
        assert!(state.started);
        state.received.push(*ctx.get_action_value(&mut sample).unwrap());
        ctx.schedule_shutdown(None);
    } }
}
#[test]
fn legacy_initializer_is_live_before_startup_and_supplied_state_stays_supported() {
    let (_, envs) = boomerang_util::runner::build_and_test_reactor_initialized(
        Input(),
        "input",
        runtime::Config::default().with_timeout(Duration::seconds(1)),
    )
    .unwrap();
    assert_eq!(INITIALIZATIONS.load(Ordering::SeqCst), 1);
    let state = envs[0]
        .reactors
        .values()
        .find_map(|r| r.get_state::<InputState>())
        .unwrap();
    assert_eq!(state.received, [42]);
    let (_, envs) = boomerang_util::runner::build_and_test_reactor(
        Input(),
        "supplied",
        InputState {
            started: false,
            received: vec![7],
        },
        runtime::Config::default()
            .with_fast_forward(true)
            .with_timeout(Duration::milliseconds(2)),
    )
    .unwrap();
    assert_eq!(INITIALIZATIONS.load(Ordering::SeqCst), 1);
    let state = envs[0]
        .reactors
        .values()
        .find_map(|r| r.get_state::<InputState>())
        .unwrap();
    assert_eq!(state.received, [7]);
}

mod generic_init {
    use super::*;

    #[derive(Debug)]
    struct GenericState;

    fn initialize<T, U, const N: usize>(
        context: GenericInit<T, U, N>,
    ) -> Result<GenericState, std::io::Error>
    where
        T: runtime::ReactorData + Copy + Default,
        U: runtime::ReactorData,
    {
        let _: runtime::AsyncActionRef<[T; N]> = context.sample;
        Ok(GenericState)
    }

    #[reactor(state = GenericState, state_init = initialize::<T, U, N>, init_context(name = GenericInit, actions = [sample]))]
    fn Generic<T, U, const N: usize>(
        #[input] _input: T,
        #[input] _unused_in_context: U,
        #[physical_action] sample: [T; N],
    ) -> impl Reactor
    where
        T: runtime::ReactorData + Copy + Default,
        U: runtime::ReactorData,
    {
        reaction! { Observe(sample) {} }
    }

    #[test]
    fn generic_context_preserves_parameters_and_bounds_in_live_initialization() {
        let (_, envs) = boomerang_util::runner::build_and_test_reactor_initialized(
            Generic::<u32, String, 2>(),
            "generic",
            runtime::Config::default()
                .with_fast_forward(true)
                .with_timeout(Duration::milliseconds(1)),
        )
        .unwrap();
        assert!(envs[0]
            .reactors
            .values()
            .any(|reactor| reactor.get_state::<GenericState>().is_some()));
    }
}
