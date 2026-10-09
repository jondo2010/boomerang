//! Manual-clock protocol, conversion, and allocation regression tests.

use super::*;
use crate::clock::RuntimeClock;

/// Creates an independent manual clock in the shared test domain.
fn clock() -> ManualClock {
    ManualClock::new(PhysicalClockDomainId(7)).unwrap()
}

/// Checks idempotent repeats, forward jumps, and retention of the first regression.
#[test]
fn repeats_jumps_and_regression_are_retained() {
    let clock = clock();
    let capability: &dyn PhysicalClock = &clock;
    assert_eq!(capability.domain(), clock.domain());
    assert_eq!(capability.epoch(), clock.epoch());
    assert_eq!(capability.now(), Ok(PhysicalInstant(0)));
    clock.advance_to(PhysicalInstant(10)).unwrap();
    clock.advance_to(PhysicalInstant(10)).unwrap();
    assert_eq!(clock.now(), Ok(PhysicalInstant(10)));
    assert_eq!(
        clock.advance_to(PhysicalInstant(9)),
        Err(PhysicalClockError::Regression)
    );
    assert_eq!(clock.now(), Err(PhysicalClockError::Regression));
    assert_eq!(
        clock.advance_to(PhysicalInstant(100)),
        Err(PhysicalClockError::Regression)
    );
    clock.close();
    assert_eq!(clock.now(), Err(PhysicalClockError::Regression));
}

/// Checks fresh run identity and terminal closure, failure, and identity rejection.
#[test]
fn closure_and_failure_are_terminal_and_epochs_are_fresh() {
    let first = clock();
    let second = clock();
    assert_ne!(first.epoch(), second.epoch());
    assert_eq!(
        first.validate(first.domain(), second.epoch()),
        Err(PhysicalClockError::EpochMismatch)
    );
    assert_eq!(
        first.validate(PhysicalClockDomainId(8), first.epoch()),
        Err(PhysicalClockError::DomainMismatch)
    );
    first.close();
    first.fail();
    assert_eq!(first.now(), Err(PhysicalClockError::Closed));
    let error: &dyn core::error::Error = &PhysicalClockError::Closed;
    assert_eq!(error.to_string(), "physical clock: Closed");
    assert!(error.source().is_none());
    second.fail();
    assert_eq!(second.now(), Err(PhysicalClockError::Failed));
}

/// Exercises registration versus advancement without lost or duplicated due wakes.
#[test]
fn registration_racing_advance_retains_one_wake_and_reuses_slot() {
    for _ in 0..200 {
        let clock = clock();
        let (tx, rx) = kanal::bounded(1);
        clock.0.state.lock().unwrap().slots.push(DeadlineSlot {
            deadline: None,
            wake: tx,
        });
        let barrier = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                barrier.wait();
                clock.advance_to(PhysicalInstant(100)).unwrap();
            });
            barrier.wait();
            if !clock.register(0, PhysicalInstant(100)).unwrap() {
                rx.recv_timeout(std::time::Duration::from_secs(1)).unwrap();
            }
        });
        assert!(rx.try_recv().unwrap().is_none());
        assert!(clock.register(0, PhysicalInstant(100)).unwrap());
        assert!(!clock.register(0, PhysicalInstant(101)).unwrap());
        assert!(!clock.register(0, PhysicalInstant(102)).unwrap());
        clock.cancel(0);
        clock.advance_to(PhysicalInstant(102)).unwrap();
        assert!(rx.try_recv().unwrap().is_none());
        clock.close();
        rx.recv().unwrap();
        assert_eq!(
            clock.register(0, PhysicalInstant(200)),
            Err(PhysicalClockError::Closed)
        );
    }
}

/// Checks conversion bounds, selected-clock action ordering, and native policy fallback.
#[test]
fn checked_conversions_and_physical_actions_use_selected_time() {
    assert_eq!(
        Tag::ZERO.checked_after(Tag::new(Duration::ZERO, usize::MAX)),
        Err(PhysicalClockError::Overflow)
    );
    use crate::{
        Action, ActionKey, ActionRef, BaseAction, CommonContext, Context, DynActionRefMut,
    };
    assert_eq!(
        PhysicalInstant(u64::MAX).checked_add(core::time::Duration::from_nanos(1)),
        Err(PhysicalClockError::Overflow)
    );
    assert_eq!(
        PhysicalInstant::from_duration(std::time::Duration::MAX),
        Err(PhysicalClockError::Overflow)
    );
    assert_eq!(
        PhysicalInstant(0).to_tag(Duration::nanoseconds(-1)),
        Err(PhysicalClockError::Overflow)
    );
    let clock = clock();
    clock.advance_to(PhysicalInstant(42)).unwrap();
    let origin = std::time::Instant::now();
    let (tx, rx) = kanal::bounded(4);
    let (_shutdown, shutdown_rx) = crate::keepalive::channel();
    let mut ctx = Context::new(
        crate::EnclaveKey::from(0),
        RuntimeClock::manual(clock.clone(), 0),
        None,
        tx,
        shutdown_rx,
    );
    ctx.physical_clock = RuntimeClock::manual(clock.clone(), 0);
    let mut action = Action::<u32>::new(
        "physical",
        ActionKey::from(0),
        Some(Duration::nanoseconds(2)),
        false,
    );
    let mut action =
        ActionRef::<u32>::try_from(DynActionRefMut(&mut action as &mut dyn BaseAction)).unwrap();
    assert_eq!(
        action.try_next_tag_for_offset(Tag::new(Duration::ZERO, usize::MAX)),
        Err(crate::ActionScheduleError::Overflow)
    );
    ctx.schedule_action(&mut action, 1, Some(Duration::nanoseconds(3)));
    assert_eq!(
        ctx.trigger_res.scheduled_actions[0].1,
        Tag::new(Duration::nanoseconds(47), 0)
    );
    let sender = ctx.make_send_context();
    let expected = PhysicalInstant(42);
    assert_eq!(ctx.try_get_physical_time().unwrap(), expected);
    assert_eq!(sender.try_get_physical_time().unwrap(), expected);
    ctx.try_schedule_action_async(&action, 2, Some(Duration::nanoseconds(3)))
        .unwrap();
    assert!(
        matches!(rx.recv().unwrap(), AsyncEvent::Physical { time, .. } if time == PhysicalInstant(47))
    );
    assert_eq!(
        ctx.try_schedule_action(&mut action, 3, Some(Duration::MIN)),
        Err(crate::ActionScheduleError::InvalidDelay)
    );
    clock.advance_to(PhysicalInstant(u64::MAX)).unwrap();
    assert_eq!(ctx.try_get_physical_time(), Ok(PhysicalInstant(u64::MAX)));
    assert_eq!(
        sender.try_get_physical_time(),
        Ok(PhysicalInstant(u64::MAX))
    );
    let scheduled = ctx.trigger_res.scheduled_actions.len();
    assert_eq!(
        ctx.try_schedule_action(&mut action, 3, None),
        Err(crate::ActionScheduleError::PhysicalClock(
            PhysicalClockError::Overflow
        ))
    );
    assert_eq!(ctx.trigger_res.scheduled_actions.len(), scheduled);
    assert_eq!(
        sender.try_schedule_action_async(&action, 3, None),
        Err(PhysicalClockError::Overflow)
    );
    assert!(rx.try_recv().unwrap().is_none());
    clock.close();
    assert_eq!(ctx.try_get_physical_time(), Err(PhysicalClockError::Closed));
    assert_eq!(
        sender.try_get_physical_time(),
        Err(PhysicalClockError::Closed)
    );
    assert_eq!(
        ctx.try_schedule_action(&mut action, 4, None),
        Err(crate::ActionScheduleError::PhysicalClock(
            PhysicalClockError::Closed
        ))
    );
    ctx.physical_clock = RuntimeClock::native(origin);
    ctx.try_schedule_action(&mut action, 5, None).unwrap();
    let tag = ctx.trigger_res.scheduled_actions.last().unwrap().1;
    assert_eq!(action.get_value_at(tag), Some(&5));
    assert!(
        ctx.make_send_context()
            .try_get_physical_time()
            .unwrap()
            .to_duration()
            <= origin.elapsed()
    );
}

/// Counts allocations on the measuring test thread while delegating to System.
struct CountingAllocator;
thread_local! { static ALLOCATIONS: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) }; }
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;
unsafe impl std::alloc::GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        ALLOCATIONS.with(|count| {
            if let Some(n) = count.get() {
                count.set(Some(n + 1));
            }
        });
        unsafe { std::alloc::System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        unsafe { std::alloc::System.dealloc(ptr, layout) }
    }
}

/// Measures allocation-free clock policies and deadline reuse at two participant scales.
#[test]
fn clock_steady_state_allocates_nothing_at_two_and_sixteen_enclaves() {
    for count in [2, 16] {
        let clock = clock();
        let channels: Vec<_> = (0..count).map(|_| kanal::bounded(1)).collect();
        clock.0.state.lock().unwrap().slots = channels
            .iter()
            .map(|(tx, _)| DeadlineSlot {
                deadline: None,
                wake: tx.clone(),
            })
            .collect();
        let adapters: Vec<_> = (0..count)
            .map(|slot| RuntimeClock::manual(clock.clone(), slot))
            .collect();
        let capacity = clock.0.state.lock().unwrap().slots.capacity();
        let start = std::time::Instant::now();
        let native = RuntimeClock::native(start);
        ALLOCATIONS.with(|n| n.set(Some(0)));
        for step in 0..10_000 {
            std::hint::black_box(native.clone())
                .now_after(std::time::Duration::ZERO)
                .unwrap();
            std::hint::black_box(RuntimeClock::manual(clock.clone(), 0));
            for adapter in &adapters {
                assert!(!adapter.failed());
                adapter.clone().now().unwrap();
            }
            for slot in 0..count {
                assert!(!clock
                    .register(slot, PhysicalInstant(step * 100 + 1))
                    .unwrap());
                clock.cancel(slot);
                assert!(!clock
                    .register(slot, PhysicalInstant(step * 100 + 2))
                    .unwrap());
            }
            clock.advance_to(PhysicalInstant(step * 100)).unwrap(); // repeated timestamp
            clock.advance_to(PhysicalInstant((step + 1) * 100)).unwrap(); // jump across all deadlines
            for (_, rx) in &channels {
                rx.try_recv().unwrap().unwrap();
                assert!(rx.try_recv().unwrap().is_none());
            }
            assert_eq!(clock.now().unwrap(), PhysicalInstant((step + 1) * 100));
        }
        let allocations = ALLOCATIONS.with(|n| n.replace(None).unwrap());
        assert_eq!(allocations, 0);
        assert_eq!(clock.0.state.lock().unwrap().slots.capacity(), capacity);
        eprintln!("clock measurement: enclaves={count} advances=10000 slots={capacity} allocations={allocations} elapsed={:?}", start.elapsed());
    }
}

/// Checks preservation of terminal failure after run cleanup.
#[test]
fn run_cleanup_retains_failure() {
    let clock = clock();
    let run = ClockRun::new(Some(&(clock.domain(), clock.clone()))).unwrap();
    clock.close(); // terminal during construction, before participant attachment
    drop(run);
    assert_eq!(clock.now(), Err(PhysicalClockError::Closed));
    let state = clock.0.state.lock().unwrap();
    assert!(state.slots.is_empty() && state.abort.is_none());
}
