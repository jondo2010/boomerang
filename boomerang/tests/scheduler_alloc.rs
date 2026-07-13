use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use boomerang::prelude::*;

struct CountingAllocator;

static COUNT_ALLOCATIONS: AtomicBool = AtomicBool::new(false);
static ALLOCATION_COUNT: AtomicUsize = AtomicUsize::new(0);
static FIRST_ALLOCATION_KIND: AtomicUsize = AtomicUsize::new(0);
static FIRST_ALLOCATION_SIZE: AtomicUsize = AtomicUsize::new(0);
static FIRST_ALLOCATION_NEW_SIZE: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if COUNT_ALLOCATIONS.load(Ordering::Relaxed)
            && ALLOCATION_COUNT.fetch_add(1, Ordering::Relaxed) == 0
        {
            FIRST_ALLOCATION_KIND.store(1, Ordering::Relaxed);
            FIRST_ALLOCATION_SIZE.store(layout.size(), Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if COUNT_ALLOCATIONS.load(Ordering::Relaxed)
            && ALLOCATION_COUNT.fetch_add(1, Ordering::Relaxed) == 0
        {
            FIRST_ALLOCATION_KIND.store(2, Ordering::Relaxed);
            FIRST_ALLOCATION_SIZE.store(layout.size(), Ordering::Relaxed);
            FIRST_ALLOCATION_NEW_SIZE.store(new_size, Ordering::Relaxed);
        }
        ptr
    }
}

#[global_allocator]
static GLOBAL: CountingAllocator = CountingAllocator;

#[derive(Debug, Default)]
struct RootActionLoopState {
    ticks: usize,
}

#[reactor(state = RootActionLoopState)]
fn RootActionLoop() -> impl Reactor {
    let ping = builder.add_logical_action::<()>("ping", None)?;
    let pong = builder.add_logical_action::<()>("pong", None)?;

    builder
        .add_reaction(Some("startup"))
        .with_startup_trigger()
        .with_effect(ping)
        .with_reaction_fn(|ctx, _state, (_startup, mut ping)| {
            ctx.schedule_action(&mut ping, (), None);
        })
        .finish()?;

    builder
        .add_reaction(Some("ping"))
        .with_trigger(ping)
        .with_effect(pong)
        .with_reaction_fn(|ctx, state, (mut ping, mut pong)| {
            assert!(ping.is_present(ctx));
            state.ticks += 1;
            ctx.schedule_action(&mut pong, (), None);
        })
        .finish()?;

    builder
        .add_reaction(Some("pong"))
        .with_trigger(pong)
        .with_effect(ping)
        .with_reaction_fn(|ctx, state, (mut pong, mut ping)| {
            assert!(pong.is_present(ctx));
            state.ticks += 1;
            ctx.schedule_action(&mut ping, (), None);
        })
        .finish()?;
}

fn build_scheduler<R, State>(reactor: R, state: State) -> runtime::Scheduler
where
    R: Reactor<State>,
    State: runtime::ReactorData,
{
    let mut assembly = Assembly::new();
    let _reactor = reactor
        .build(
            "root_action_loop",
            state,
            None,
            None,
            None,
            false,
            &mut assembly,
        )
        .unwrap();
    let config = runtime::Config::default().with_fast_forward(true);
    let BuilderRuntimeParts { enclaves, .. } = assembly.into_runtime_parts(&config).unwrap();
    let (enclave_key, enclave) = enclaves.into_iter().next().unwrap();
    runtime::Scheduler::new(enclave_key, enclave, config)
}

fn start_counting_allocations() {
    ALLOCATION_COUNT.store(0, Ordering::Relaxed);
    FIRST_ALLOCATION_KIND.store(0, Ordering::Relaxed);
    FIRST_ALLOCATION_SIZE.store(0, Ordering::Relaxed);
    FIRST_ALLOCATION_NEW_SIZE.store(0, Ordering::Relaxed);
    COUNT_ALLOCATIONS.store(true, Ordering::Relaxed);
}

fn stop_counting_allocations() -> usize {
    COUNT_ALLOCATIONS.store(false, Ordering::Relaxed);
    ALLOCATION_COUNT.load(Ordering::Relaxed)
}

#[test]
#[cfg_attr(
    feature = "parallel",
    ignore = "the parallel scheduler path uses Rayon/thread-pool machinery that allocates"
)]
fn steady_state_root_action_scheduler_next_does_not_allocate() {
    let mut scheduler = build_scheduler(RootActionLoop(), RootActionLoopState::default());
    scheduler.startup();

    for _ in 0..4096 {
        assert!(scheduler.try_next().unwrap());
    }

    start_counting_allocations();
    for _ in 0..1024 {
        assert!(scheduler.try_next().unwrap());
    }
    let allocations = stop_counting_allocations();

    assert_eq!(
        allocations, 0,
        "steady-state non-modal root action scheduling allocated {allocations} times after warmup; first kind={}, first size={}, first new size={}",
        FIRST_ALLOCATION_KIND.load(Ordering::Relaxed),
        FIRST_ALLOCATION_SIZE.load(Ordering::Relaxed),
        FIRST_ALLOCATION_NEW_SIZE.load(Ordering::Relaxed)
    );
}
