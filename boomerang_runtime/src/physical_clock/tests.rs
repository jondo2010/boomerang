use super::*;

fn clock() -> ManualClock {
    ManualClock::new(PhysicalClockDomainId(7)).unwrap()
}

#[test]
fn repeats_jumps_and_regression_are_retained() {
    let clock = clock();
    assert_eq!(clock.now(), Ok(PhysicalTimeNanos(0)));
    clock.advance_to(PhysicalTimeNanos(10)).unwrap();
    clock.advance_to(PhysicalTimeNanos(10)).unwrap();
    assert_eq!(clock.now(), Ok(PhysicalTimeNanos(10)));
    assert_eq!(
        clock.advance_to(PhysicalTimeNanos(9)),
        Err(PhysicalClockError::Regression)
    );
    assert_eq!(clock.now(), Err(PhysicalClockError::Regression));
    assert_eq!(
        clock.advance_to(PhysicalTimeNanos(100)),
        Err(PhysicalClockError::Regression)
    );
    clock.close();
    assert_eq!(clock.now(), Err(PhysicalClockError::Regression));
}

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
    second.fail();
    assert_eq!(second.now(), Err(PhysicalClockError::Failed));
}

#[test]
fn registration_racing_advance_retains_one_wake_and_reuses_slot() {
    for _ in 0..200 {
        let clock = clock();
        let (tx, rx) = kanal::bounded(1);
        clock.0.state.lock().unwrap().slots.push(Slot {
            deadline: None,
            wake: tx,
        });
        let barrier = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                barrier.wait();
                clock.advance_to(PhysicalTimeNanos(100)).unwrap();
            });
            barrier.wait();
            if !clock.register(0, PhysicalTimeNanos(100)).unwrap() {
                rx.recv_timeout(std::time::Duration::from_secs(1)).unwrap();
            }
        });
        assert!(rx.try_recv().unwrap().is_none());
        assert!(clock.register(0, PhysicalTimeNanos(100)).unwrap());
        assert!(!clock.register(0, PhysicalTimeNanos(101)).unwrap());
        assert!(!clock.register(0, PhysicalTimeNanos(102)).unwrap());
        clock.cancel(0);
        clock.advance_to(PhysicalTimeNanos(102)).unwrap();
        assert!(rx.try_recv().unwrap().is_none());
        clock.close();
        rx.recv().unwrap();
        assert_eq!(
            clock.register(0, PhysicalTimeNanos(200)),
            Err(PhysicalClockError::Closed)
        );
    }
}

#[test]
fn checked_conversions_and_physical_actions_use_selected_time() {
    use crate::{
        Action, ActionKey, ActionRef, BaseAction, CommonContext, Context, DynActionRefMut,
    };
    assert_eq!(
        PhysicalTimeNanos(u64::MAX).checked_add(PhysicalTimeNanos(1)),
        Err(PhysicalClockError::Overflow)
    );
    assert_eq!(
        PhysicalTimeNanos::from_duration(std::time::Duration::MAX),
        Err(PhysicalClockError::Overflow)
    );
    assert_eq!(
        PhysicalTimeNanos(0).to_tag(Duration::nanoseconds(-1)),
        Err(PhysicalClockError::Overflow)
    );
    let clock = clock();
    clock.advance_to(PhysicalTimeNanos(42)).unwrap();
    let origin = std::time::Instant::now();
    let (tx, rx) = kanal::bounded(4);
    let (_shutdown, shutdown_rx) = crate::keepalive::channel();
    let mut ctx = Context::new(crate::EnclaveKey::from(0), origin, None, tx, shutdown_rx);
    ctx.physical_clock = Some(ClockContext {
        clock: clock.clone(),
        slot: 0,
        origin,
    });
    let mut action = Action::<u32>::new(
        "physical",
        ActionKey::from(0),
        Some(Duration::nanoseconds(2)),
        false,
    );
    let mut action =
        ActionRef::<u32>::try_from(DynActionRefMut(&mut action as &mut dyn BaseAction)).unwrap();
    ctx.try_schedule_action(&mut action, 1, Some(Duration::nanoseconds(3)))
        .unwrap();
    assert_eq!(
        ctx.trigger_res.scheduled_actions[0].1,
        Tag::new(Duration::nanoseconds(47), 0)
    );
    let sender = ctx.make_send_context();
    assert_eq!(sender.physical_time().unwrap(), Some(PhysicalTimeNanos(42)));
    sender
        .try_schedule_action_async(&action, 2, Some(Duration::nanoseconds(3)))
        .unwrap();
    assert!(
        matches!(rx.recv().unwrap(), AsyncEvent::Physical { time, .. } if time == origin + std::time::Duration::from_nanos(47))
    );
    assert_eq!(
        ctx.try_schedule_action(&mut action, 3, Some(Duration::MIN)),
        Err(PhysicalClockError::Overflow)
    );
    clock.advance_to(PhysicalTimeNanos(u64::MAX)).unwrap();
    assert_eq!(
        sender.try_schedule_action_async(&action, 3, None),
        Err(PhysicalClockError::Overflow)
    );
    clock.close();
    assert_eq!(
        sender.try_get_physical_time(),
        Err(PhysicalClockError::Closed)
    );
    assert_eq!(
        ctx.try_schedule_action(&mut action, 4, None),
        Err(PhysicalClockError::Closed)
    );
}

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

#[test]
fn clock_steady_state_allocates_nothing_at_two_and_sixteen_enclaves() {
    for count in [2, 16] {
        let clock = clock();
        let channels: Vec<_> = (0..count).map(|_| kanal::bounded(1)).collect();
        clock.0.state.lock().unwrap().slots = channels
            .iter()
            .map(|(tx, _)| Slot {
                deadline: None,
                wake: tx.clone(),
            })
            .collect();
        let capacity = clock.0.state.lock().unwrap().slots.capacity();
        let start = std::time::Instant::now();
        ALLOCATIONS.with(|n| n.set(Some(0)));
        for step in 0..10_000 {
            for slot in 0..count {
                assert!(!clock
                    .register(slot, PhysicalTimeNanos(step * 100 + 1))
                    .unwrap());
                clock.cancel(slot);
                assert!(!clock
                    .register(slot, PhysicalTimeNanos(step * 100 + 2))
                    .unwrap());
            }
            clock.advance_to(PhysicalTimeNanos(step * 100)).unwrap(); // repeated timestamp
            clock
                .advance_to(PhysicalTimeNanos((step + 1) * 100))
                .unwrap(); // jump across all deadlines
            for (_, rx) in &channels {
                rx.try_recv().unwrap().unwrap();
                assert!(rx.try_recv().unwrap().is_none());
            }
            assert_eq!(clock.now().unwrap(), PhysicalTimeNanos((step + 1) * 100));
        }
        let allocations = ALLOCATIONS.with(|n| n.replace(None).unwrap());
        assert_eq!(allocations, 0);
        assert_eq!(clock.0.state.lock().unwrap().slots.capacity(), capacity);
        eprintln!("clock measurement: enclaves={count} advances=10000 slots={capacity} allocations={allocations} elapsed={:?}", start.elapsed());
    }
}

#[test]
fn host_clock_uses_shared_integer_contract_and_run_cleanup_retains_failure() {
    let origin = std::time::Instant::now();
    let host = HostClock::new(PhysicalClockDomainId(7), origin).unwrap();
    let capability: &dyn PhysicalClock = &host;
    assert_eq!(capability.domain(), PhysicalClockDomainId(7));
    assert_ne!(capability.epoch(), clock().epoch());
    assert!(capability.now().unwrap().to_instant(origin).unwrap() <= std::time::Instant::now());
    let clock = clock();
    let run = ClockRun::new(Some(&(clock.domain(), clock.clone()))).unwrap();
    clock.close(); // terminal during construction, before participant attachment
    drop(run);
    assert_eq!(clock.now(), Err(PhysicalClockError::Closed));
    let state = clock.0.state.lock().unwrap();
    assert!(state.slots.is_empty() && state.abort.is_none());
}
