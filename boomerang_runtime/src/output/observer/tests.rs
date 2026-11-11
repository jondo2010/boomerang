use super::*;
use crate::image::{EnclaveIndex, ReactorIndex};
use crate::sched::output::OutputState;

fn fixture(capacity: usize) -> (OutputState, OutputObserver, crate::Receiver<AsyncEvent>) {
    let reactor = ReactorIndex::new(0);
    let mut state = OutputState::new([reactor].into_iter());
    let (sender, receiver) = kanal::bounded(capacity);
    state
        .configure(EnclaveIndex::new(0), ExecutionEpoch(7), sender)
        .unwrap();
    let observer = state.observer(reactor).unwrap();
    (state, observer, receiver)
}

fn accept_next(state: &mut OutputState, receiver: &crate::Receiver<AsyncEvent>) {
    let AsyncEvent::OutputRequest(request) = receiver.recv().unwrap() else {
        panic!("expected an output request");
    };
    state.accept(*request);
}

#[test]
fn output_lifecycle_keeps_the_exact_first_error() {
    let lifecycle = Lifecycle::default();
    lifecycle.close(OutputWaitError::StaleExecution);
    lifecycle.close(OutputWaitError::ExecutionFailed);
    assert_eq!(lifecycle.error(), Some(OutputWaitError::StaleExecution));
}

#[test]
fn output_request_capacity_includes_an_unread_reply() {
    let (mut state, observer, receiver) = fixture(1);
    let wait = observer.request_tag(Tag::ZERO).unwrap();
    accept_next(&mut state, &receiver);
    state.complete_bound(OutputBound::ThroughTag(Tag::ZERO));
    assert!(matches!(
        observer.request_tag(Tag::ZERO),
        Err(OutputWaitError::Capacity)
    ));
    assert_eq!(
        wait.wait().unwrap().bound(),
        OutputBound::ThroughTag(Tag::ZERO)
    );
    // Reading the reply releases its slot and must not leave a needless wake in the mailbox.
    assert!(observer.request_tag(Tag::ZERO).is_ok());
}

#[test]
fn output_request_send_failure_releases_its_slot() {
    let (_state, observer, receiver) = fixture(1);
    observer
        .endpoint
        .event_tx
        .try_send(AsyncEvent::OutputWake)
        .unwrap();
    assert!(matches!(
        observer.request_tag(Tag::ZERO),
        Err(OutputWaitError::Capacity)
    ));
    assert!(matches!(receiver.recv().unwrap(), AsyncEvent::OutputWake));
    assert!(observer.request_tag(Tag::ZERO).is_ok());
}

#[test]
fn output_queued_request_does_not_keep_its_mailbox_alive() {
    let (state, observer, receiver) = fixture(1);
    let lifecycle = Arc::downgrade(&observer.endpoint.lifecycle);
    let wait = observer.request_tag(Tag::ZERO).unwrap();
    drop(state);
    drop(observer);
    drop(wait);
    drop(receiver);
    assert!(
        lifecycle.upgrade().is_none(),
        "queued requests must not own mailbox senders"
    );
}

#[cfg(feature = "hosted-manual-clock")]
#[test]
fn output_epoch_stays_fixed_after_all_observers_are_dropped() {
    let (mut state, observer, _receiver) = fixture(1);
    let epoch = observer.epoch();
    drop(observer);
    let clock = crate::clock::ManualClock::new(crate::clock::PhysicalClockDomainId(3)).unwrap();
    assert_eq!(
        state.select_clock(&crate::clock::ExecutionClock::manual(clock, 0)),
        Err(crate::clock::PhysicalClockError::AlreadyUsed)
    );
    let next = state.observer(ReactorIndex::new(0)).unwrap();
    assert_eq!(next.epoch(), epoch);
}

#[test]
fn output_issued_proof_survives_later_termination() {
    let (mut state, observer, receiver) = fixture(1);
    let wait = observer.request_tag(Tag::ZERO).unwrap();
    accept_next(&mut state, &receiver);
    state.complete_bound(OutputBound::ThroughTag(Tag::ZERO));
    state.close(OutputWaitError::ExecutionFailed);
    assert_eq!(
        wait.wait().unwrap().bound(),
        OutputBound::ThroughTag(Tag::ZERO)
    );
    assert!(matches!(
        observer.request_tag(Tag::ZERO),
        Err(OutputWaitError::ExecutionFailed)
    ));
}

#[test]
fn output_reply_after_empty_poll_precedes_lifecycle_termination() {
    let (mut state, observer, receiver) = fixture(1);
    let wait = observer.request_tag(Tag::ZERO).unwrap();
    accept_next(&mut state, &receiver);
    let completion = wait
        .receive(Some(Duration::from_secs(1)), || {
            state.complete_bound(OutputBound::ThroughTag(Tag::ZERO));
            state.close(OutputWaitError::ExecutionFailed);
        })
        .unwrap();
    assert_eq!(completion.bound(), OutputBound::ThroughTag(Tag::ZERO));
    assert_eq!(completion.epoch(), ExecutionEpoch(7));
}

#[test]
fn output_reply_after_empty_poll_precedes_mailbox_disconnect() {
    let (mut state, observer, receiver) = fixture(1);
    let wait = observer.request_tag(Tag::ZERO).unwrap();
    accept_next(&mut state, &receiver);
    let completion = wait
        .receive(Some(Duration::from_secs(1)), || {
            state.complete_bound(OutputBound::ThroughTag(Tag::ZERO));
            receiver.close().unwrap();
        })
        .unwrap();
    assert_eq!(completion.bound(), OutputBound::ThroughTag(Tag::ZERO));
    assert_eq!(completion.epoch(), ExecutionEpoch(7));
}

#[test]
fn output_reply_after_empty_poll_precedes_expired_timeout() {
    let (mut state, observer, receiver) = fixture(1);
    let wait = observer.request_tag(Tag::ZERO).unwrap();
    accept_next(&mut state, &receiver);
    let completion = wait
        .receive(Some(Duration::ZERO), || {
            state.complete_bound(OutputBound::ThroughTag(Tag::ZERO));
        })
        .unwrap();
    assert_eq!(completion.bound(), OutputBound::ThroughTag(Tag::ZERO));
    assert_eq!(completion.epoch(), ExecutionEpoch(7));
}

#[test]
fn output_reply_after_empty_poll_keeps_exact_request_error() {
    let (mut state, observer, receiver) = fixture(1);
    let wait = observer.request_tag(Tag::ZERO).unwrap();
    let AsyncEvent::OutputRequest(request) = receiver.recv().unwrap() else {
        panic!("expected an output request");
    };
    let mut request = Some(request);
    let result = wait.receive(Some(Duration::from_secs(1)), || {
        request
            .take()
            .unwrap()
            .reply(Err(OutputWaitError::InvalidBound));
        state.close(OutputWaitError::ExecutionFailed);
    });
    assert_eq!(result, Err(OutputWaitError::InvalidBound));
}

#[test]
fn output_empty_poll_without_reply_keeps_terminal_errors() {
    for expected in [
        OutputWaitError::StaleExecution,
        OutputWaitError::Disconnected,
        OutputWaitError::Timeout,
    ] {
        let (mut state, observer, receiver) = fixture(1);
        // Leave the request queued so lifecycle closure cannot send a reply.
        let wait = observer.request_tag(Tag::ZERO).unwrap();
        let result = wait.receive(Some(Duration::ZERO), || match expected {
            OutputWaitError::StaleExecution => {
                state.close(OutputWaitError::StaleExecution);
                state.close(OutputWaitError::ExecutionFailed);
            }
            OutputWaitError::Disconnected => {
                receiver.close().unwrap();
            }
            OutputWaitError::Timeout => {}
            _ => unreachable!(),
        });
        assert_eq!(result, Err(expected));
    }
}
