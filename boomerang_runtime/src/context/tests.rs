//! Tests for reaction scheduling and external event delivery.

use super::*;
#[cfg(feature = "hosted-manual-clock")]
use crate::clock::PhysicalInstant;
use crate::{
    clock::PhysicalClockError, keepalive, Action, ActionKey, ActionRef, AsyncActionRef, AsyncEvent,
    BaseAction, Duration, DynActionRef, DynActionRefMut, EnclaveKey, Tag,
};

/// Check that a rejected logical action leaves existing values and triggers unchanged.
fn assert_logical_rejected(
    current: Tag,
    minimum: Duration,
    delay: Duration,
    expected: ActionScheduleError,
) {
    let (tx, _rx) = kanal::unbounded();
    let (_shutdown, shutdown_rx) = keepalive::channel();
    let mut ctx = Context::new(
        EnclaveKey::from(0),
        crate::clock::ExecutionClock::host_monotonic(std::time::Instant::now()),
        None,
        tx,
        shutdown_rx,
    );
    ctx.reset_for_reaction(current);
    let mut action = Action::<u32>::new("logical", ActionKey::from(0), Some(minimum), true);
    let mut action =
        ActionRef::<u32>::try_from(DynActionRefMut(&mut action as &mut dyn BaseAction)).unwrap();
    action.set_value(Tag::ZERO, 7);
    assert_eq!(
        ctx.try_schedule_action(&mut action, 9, Some(delay)),
        Err(expected)
    );
    assert!(ctx.trigger_res.scheduled_actions.is_empty());
    assert_eq!(action.get_value_at(Tag::ZERO), Some(&7));
}

/// A negative total delay does not schedule an event.
#[test]
fn checked_logical_rejects_negative_delay() {
    assert_logical_rejected(
        Tag::new(Duration::nanoseconds(10), 0),
        Duration::ZERO,
        Duration::nanoseconds(-1),
        ActionScheduleError::InvalidDelay,
    );
}

/// An overflow in the delay sum returns an error.
#[test]
fn checked_logical_rejects_delay_sum_overflow() {
    assert_logical_rejected(
        Tag::ZERO,
        Duration::nanoseconds(1),
        Duration::MAX,
        ActionScheduleError::Overflow,
    );
}

/// A valid delay can still exceed the remaining range of the logical offset.
#[test]
fn checked_logical_rejects_offset_overflow() {
    assert_logical_rejected(
        Tag::new(Duration::MAX, 0),
        Duration::ZERO,
        Duration::nanoseconds(1),
        ActionScheduleError::Overflow,
    );
}

/// Zero-delay scheduling cannot wrap an exhausted current microstep.
#[test]
fn checked_logical_rejects_microstep_overflow() {
    assert_logical_rejected(
        Tag::new(Duration::ZERO, usize::MAX),
        Duration::ZERO,
        Duration::ZERO,
        ActionScheduleError::Overflow,
    );
}

/// An exhausted microstep cursor returns an error before a value is stored.
#[test]
fn checked_logical_rejects_saturated_cursor() {
    assert_logical_rejected(
        Tag::new(Duration::ZERO, usize::MAX - 1),
        Duration::ZERO,
        Duration::ZERO,
        ActionScheduleError::Overflow,
    );
}

/// Logical delays retain their full range and repeated values retain separate microsteps.
#[test]
fn checked_logical_preserves_large_offsets_and_values() {
    let (tx, _rx) = kanal::unbounded();
    let (_shutdown, shutdown_rx) = keepalive::channel();
    let mut ctx = Context::new(
        EnclaveKey::from(0),
        crate::clock::ExecutionClock::host_monotonic(std::time::Instant::now()),
        None,
        tx,
        shutdown_rx,
    );
    ctx.reset_for_reaction(Tag::new(Duration::nanoseconds(10), 3));
    let mut action = Action::<u32>::new(
        "logical",
        ActionKey::from(0),
        Some(Duration::nanoseconds(2)),
        true,
    );
    let mut action =
        ActionRef::<u32>::try_from(DynActionRefMut(&mut action as &mut dyn BaseAction)).unwrap();
    let delay = Duration::seconds(20_000_000_000);
    ctx.try_schedule_action(&mut action, 1, Some(delay))
        .unwrap();
    ctx.schedule_action(&mut action, 2, Some(delay));
    let offset = Duration::seconds(20_000_000_000) + Duration::nanoseconds(12);
    assert_eq!(
        ctx.trigger_res.scheduled_actions,
        [
            (ActionKey::from(0), Tag::new(offset, 0)),
            (ActionKey::from(0), Tag::new(offset, 1))
        ]
    );
    assert_eq!(action.get_value_at(Tag::new(offset, 0)), Some(&1));
    assert_eq!(action.get_value_at(Tag::new(offset, 1)), Some(&2));
    ctx.reset_for_reaction(Tag::new(offset, 1));
    assert_eq!(ctx.get_elapsed_logical_time(), offset);
    assert_eq!(
        ctx.try_get_logical_time(),
        Err(crate::clock::PhysicalClockError::Overflow)
    );
    ctx.reset_for_reaction(Tag::new(Duration::nanoseconds(42), 7));
    assert_eq!(
        ctx.try_get_logical_time(),
        Ok(crate::clock::PhysicalInstant(42))
    );
}

/// Physical actions advance past the current logical tag, even when it exceeds physical time.
#[test]
fn physical_actions_advance_past_current_tag_with_checked_overflow() {
    let (tx, _rx) = kanal::unbounded();
    let (_shutdown, shutdown_rx) = keepalive::channel();
    let mut ctx = Context::new(
        EnclaveKey::from(0),
        crate::clock::ExecutionClock::host_monotonic(std::time::Instant::now()),
        None,
        tx,
        shutdown_rx,
    );
    let offset = Duration::seconds(1_000_000_000);
    ctx.reset_for_reaction(Tag::new(offset, 7));
    let mut action = Action::<u32>::new("physical", ActionKey::from(0), None, false);
    let mut action =
        ActionRef::<u32>::try_from(DynActionRefMut(&mut action as &mut dyn BaseAction)).unwrap();
    ctx.try_schedule_action(&mut action, 1, None).unwrap();
    ctx.try_schedule_action(&mut action, 2, None).unwrap();
    assert_eq!(ctx.trigger_res.scheduled_actions[0].1, Tag::new(offset, 8));
    assert_eq!(ctx.trigger_res.scheduled_actions[1].1, Tag::new(offset, 9));
    ctx.reset_for_reaction(Tag::new(offset, usize::MAX));
    assert_eq!(
        ctx.try_schedule_action(&mut action, 3, None),
        Err(ActionScheduleError::Overflow)
    );
    assert!(ctx.trigger_res.scheduled_actions.is_empty());
    assert_eq!(action.get_value_at(Tag::new(offset, 8)), Some(&1));
    assert_eq!(action.get_value_at(Tag::new(offset, 9)), Some(&2));
}

#[test]
fn schedule_action_advances_microsteps_for_same_delay() {
    let (async_tx, _async_rx) = kanal::unbounded::<AsyncEvent>();
    let (_shutdown_tx, shutdown_rx) = keepalive::channel();

    let mut ctx = Context::new(
        EnclaveKey::from(0),
        crate::clock::ExecutionClock::host_monotonic(std::time::Instant::now()),
        None,
        async_tx,
        shutdown_rx,
    );

    ctx.reset_for_reaction(Tag::ZERO);

    let mut action = Action::<u32>::new("test", ActionKey::from(0), None, true);
    let mut action_ref =
        ActionRef::<u32>::try_from(DynActionRefMut(&mut action as &mut dyn BaseAction))
            .expect("action ref");

    ctx.schedule_action(&mut action_ref, 1, None);
    ctx.try_schedule_action(&mut action_ref, 2, None).unwrap();

    let tags: Vec<Tag> = ctx
        .trigger_res
        .scheduled_actions
        .iter()
        .map(|(_, tag)| *tag)
        .collect();

    assert_eq!(tags.len(), 2);
    assert_eq!(tags[0].offset(), tags[1].offset());
    assert_eq!(tags[1].microstep(), tags[0].microstep() + 1);
}

#[test]
fn nonblocking_sends_distinguish_acceptance_backpressure_and_disconnection() {
    let (async_tx, async_rx) = kanal::bounded::<AsyncEvent>(1);
    let (_shutdown_tx, shutdown_rx) = keepalive::channel();
    let ctx = Context::new(
        EnclaveKey::from(0),
        crate::clock::ExecutionClock::host_monotonic(std::time::Instant::now()),
        None,
        async_tx,
        shutdown_rx,
    );
    let sender = ctx.make_send_context();
    let event = || AsyncEvent::shutdown(Duration::ZERO);

    assert_eq!(ctx.try_schedule_async(event()), Some(true));
    assert_eq!(ctx.try_schedule_async(event()), None);
    assert_eq!(sender.try_schedule_async(event()), None);
    assert!(matches!(
        async_rx.recv().unwrap(),
        AsyncEvent::Shutdown { .. }
    ));
    assert_eq!(sender.try_schedule_async(event()), Some(true));
    assert!(matches!(
        async_rx.recv().unwrap(),
        AsyncEvent::Shutdown { .. }
    ));
    drop(async_rx);
    assert_eq!(ctx.try_schedule_async(event()), Some(false));
    assert_eq!(sender.try_schedule_async(event()), Some(false));
}

#[test]
fn send_context_reports_closed_scheduler_when_requesting_shutdown() {
    let (async_tx, async_rx) = kanal::unbounded::<AsyncEvent>();
    drop(async_rx);
    let (_shutdown_tx, shutdown_rx) = keepalive::channel();
    let mut ctx = SendContext {
        clock: Default::default(),
        enclave_key: EnclaveKey::from(0),
        async_tx,
        shutdown_rx,
    };

    assert!(!ctx.schedule_shutdown(None));
}

#[derive(Debug)]
struct Payload(Box<u32>);

#[test]
fn prepared_full_retry_retains_payload_allocation_and_time() {
    let (tx, rx) = kanal::bounded(1);
    let (_keepalive, shutdown_rx) = keepalive::channel();
    let send = SendContext {
        clock: crate::clock::ExecutionClock::host_monotonic(std::time::Instant::now()),
        enclave_key: EnclaveKey::new(0),
        async_tx: tx,
        shutdown_rx,
    };
    let action = Action::<Payload>::new(
        "input",
        ActionKey::new(0),
        Some(Duration::milliseconds(7)),
        false,
    );
    let action = AsyncActionRef::<Payload>::try_from(DynActionRef(&action)).unwrap();
    send.prepare_action(&action, Payload(Box::new(1)), None)
        .unwrap()
        .try_send()
        .unwrap();
    let payload = Payload(Box::new(42));
    let allocation = payload.0.as_ref() as *const u32;
    let prepared = send
        .prepare_action(&action, payload, Some(Duration::milliseconds(3)))
        .unwrap();
    let prepared_time = prepared.time();
    let mut prepared = prepared;
    for _ in 0..8 {
        prepared = match prepared.try_send() {
            Err(TrySendError::Full(value)) => value,
            other => panic!("full mailbox must retain prepared input: {other:?}"),
        };
        assert_eq!(prepared.time(), prepared_time);
    }
    rx.recv().unwrap();
    prepared.try_send().unwrap();
    let AsyncEvent::Physical { time, value, .. } = rx.recv().unwrap() else {
        panic!("physical input")
    };
    let payload = value.downcast::<Payload>().ok().unwrap();
    assert_eq!(payload.0.as_ref() as *const u32, allocation);
    assert_eq!(*payload.0, 42);
    assert_eq!(time, prepared_time);
    assert!(
        time >= crate::clock::PhysicalInstant::from_duration(std::time::Duration::from_millis(10))
            .unwrap()
    );
    let prepared = send
        .prepare_action(&action, Payload(Box::new(3)), None)
        .unwrap();
    rx.close().unwrap();
    assert!(matches!(prepared.try_send(), Err(TrySendError::Closed(_))));
}
#[test]
fn preparation_clock_and_arithmetic_failures_publish_nothing() {
    let (tx, rx) = kanal::bounded(1);
    let (_keepalive, shutdown_rx) = keepalive::channel();
    let mut sender = SendContext {
        clock: crate::clock::ExecutionClock::default(),
        enclave_key: EnclaveKey::new(0),
        async_tx: tx,
        shutdown_rx,
    };
    let action = Action::<u32>::new(
        "input",
        ActionKey::new(0),
        Some(Duration::nanoseconds(2)),
        false,
    );
    let action = AsyncActionRef::<u32>::try_from(DynActionRef(&action)).unwrap();
    assert!(matches!(
        sender.prepare_action(&action, 1, None),
        Err(PhysicalClockError::NotStarted)
    ));
    sender.clock = crate::clock::ExecutionClock::host_monotonic(std::time::Instant::now());
    assert!(matches!(
        sender.prepare_action(&action, 1, Some(Duration::MAX)),
        Err(PhysicalClockError::Overflow)
    ));
    assert!(matches!(
        sender.prepare_action(&action, 1, Some(Duration::MIN)),
        Err(PhysicalClockError::Overflow)
    ));
    assert!(rx.try_recv().unwrap().is_none());
}
#[cfg(feature = "hosted-manual-clock")]
#[test]
fn manual_time_advance_does_not_retime_or_allocate_on_retry() {
    use crate::clock::{ExecutionClock, ManualClock, PhysicalClockDomainId};
    let clock = ManualClock::new(PhysicalClockDomainId(7)).unwrap();
    clock.advance_to(PhysicalInstant(42)).unwrap();
    let (tx, rx) = kanal::bounded(1);
    let (_keepalive, shutdown_rx) = keepalive::channel();
    let sender = SendContext {
        clock: ExecutionClock::manual(clock.clone(), 0),
        enclave_key: EnclaveKey::new(0),
        async_tx: tx,
        shutdown_rx,
    };
    let action = Action::<u32>::new(
        "input",
        ActionKey::new(0),
        Some(Duration::nanoseconds(2)),
        false,
    );
    let action = AsyncActionRef::<u32>::try_from(DynActionRef(&action)).unwrap();
    sender
        .try_schedule_action_async(&action, 1, Some(Duration::nanoseconds(3)))
        .unwrap();
    let prepared = sender
        .prepare_action(&action, 2, Some(Duration::nanoseconds(3)))
        .unwrap();
    crate::clock::ALLOCATIONS.with(|count| count.set(Some(0)));
    let mut prepared = prepared;
    for _ in 0..16 {
        prepared = match prepared.try_send() {
            Err(TrySendError::Full(retry)) => retry,
            other => panic!("expected backpressure: {other:?}"),
        };
    }
    let allocations = crate::clock::ALLOCATIONS.with(|count| count.replace(None).unwrap());
    assert_eq!(allocations, 0);
    clock.advance_to(PhysicalInstant(100)).unwrap();
    let AsyncEvent::Physical { time, .. } = rx.recv().unwrap() else {
        panic!("physical input")
    };
    assert_eq!(
        time,
        PhysicalInstant(47),
        "blocking compatibility helper shares preparation"
    );
    prepared.try_send().unwrap();
    let AsyncEvent::Physical { time, .. } = rx.recv().unwrap() else {
        panic!("physical input")
    };
    assert_eq!(
        time,
        PhysicalInstant(47),
        "retry keeps acquisition time plus declared and requested delays"
    );
    clock.advance_to(PhysicalInstant(u64::MAX)).unwrap();
    assert!(matches!(
        sender.prepare_action(&action, 3, None),
        Err(PhysicalClockError::Overflow)
    ));
    clock.fail();
    assert!(matches!(
        sender.prepare_action(&action, 4, None),
        Err(PhysicalClockError::Failed)
    ));
    assert!(rx.try_recv().unwrap().is_none());
}
