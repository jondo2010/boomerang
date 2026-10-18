//! Source protocol validation and batch publication.
//!
//! The coordinator owns validation, source progress, and publication state.
//! The coordinator owns execution authority and publication fences.
//! A partially published batch stops execution before admission requests an abort.
//! Required-source protocol errors, overflow, and disconnection also abort execution.
//!
//! Producers lease one bounded request lane. Owned batches carry their progress.
//! The coordinator replies through one reusable channel, separate from scheduler
//! command and final-permit replies. The report-queue wake can allocate; this
//! transport does not make complete submissions allocation-free.

use super::{
    retention::{Reservation, RetainedBatches},
    CompiledPhysicalAction, InputBatch, InputConfig, InputError, InputErrorKind, InputObservation,
    InputReceipt, InputSourceHandle, SourceKey,
};
use crate::{
    clock::{PhysicalInstant, SelectedClock},
    image::EnclaveIndex,
    sched::federate::state::FederateCoordinationState,
    AsyncEvent, Duration, Tag,
};
use std::{
    sync::{Arc, OnceLock, Weak},
    time::Instant,
};

/// Per-source sequence, acquisition time, exclusive frontier, and minimum target delay.
struct SourceState {
    actions: Vec<CompiledPhysicalAction>,
    id: String,
    required: bool,
    enabled: bool,
    last: Option<(u64, PhysicalInstant)>,
    frontier: Option<PhysicalInstant>,
    delay: Duration,
}

/// Tentative source updates. Copy these into source state only after publication succeeds.
#[derive(Clone, Copy, Default)]
struct PendingSource {
    last: Option<(u64, PhysicalInstant)>,
    frontier: Option<PhysicalInstant>,
}

/// Reusable validation storage. Its capacities are fixed before execution starts.
struct AdmissionScratch {
    sources: tinymap::TinyMap<SourceKey, PendingSource>,
    reservations: Vec<Reservation>,
}

impl AdmissionScratch {
    /// Discard tentative updates from the previous submission.
    fn reset(&mut self, sources: &tinymap::TinyMap<SourceKey, SourceState>) {
        for (pending, source) in self.sources.values_mut().zip(sources.values()) {
            pending.last = source.last;
            pending.frontier = source.frontier;
        }
        self.reservations.clear();
    }
}

/// Source protocol state and retained-batch counts. The coordinator owns execution authority.
struct AdmissionState {
    sources: tinymap::TinyMap<SourceKey, SourceState>,
    retained: RetainedBatches,
    scratch: AdmissionScratch,
    horizon: Tag,
    failure: Option<InputError>,
}
/// Immutable execution identity and publication endpoints.
struct AdmissionEngine {
    clock: SelectedClock,
    endpoint: Weak<AdmissionService>,
    senders: tinymap::TinySecondaryMap<EnclaveIndex, crate::Sender<AsyncEvent>>,
    max_values: usize,
    max_batches: usize,
    abort: crate::sched::federate::FederateAbortHandle,
}

/// Single-owner protocol processor. Only the coordinator processes requests.
pub(crate) struct AdmissionProcessor {
    engine: AdmissionEngine,
    state: AdmissionState,
    requests: crate::Receiver<AdmissionRequest>,
    replies: crate::Sender<Result<InputReceipt, InputError>>,
    leases: crate::Sender<()>,
    failure: Arc<OnceLock<InputError>>,
}

/// One owned request occupies the reusable synchronous lane.
struct AdmissionRequest {
    arrival: Instant,
    operation: AdmissionOperation,
}

/// Request payload owns every value that crosses the thread boundary.
enum AdmissionOperation {
    Submit(InputBatch),
    Advance(InputSourceHandle, PhysicalInstant),
    Disconnect(InputSourceHandle),
}

/// Channel endpoints and immutable execution metadata. No protocol state is shared.
pub(super) struct AdmissionService {
    pub(super) clock: SelectedClock,
    sources: tinymap::TinyMap<SourceKey, ()>,
    requests: crate::Sender<AdmissionRequest>,
    replies: crate::Receiver<Result<InputReceipt, InputError>>,
    leases: crate::Receiver<()>,
    lease_return: crate::Sender<()>,
    failure: Arc<OnceLock<InputError>>,
    notify: crate::sched::federate::FederateAbortHandle,
}

/// A synchronous producer endpoint for one execution. Clones share one bounded request lane.
#[derive(Clone)]
pub struct InputAdmission(pub(super) Arc<AdmissionService>);

impl std::fmt::Debug for InputAdmission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InputAdmission").finish_non_exhaustive()
    }
}

impl InputAdmission {
    /// Bind a compiled source index to this execution.
    pub(crate) fn source_handle(&self, key: SourceKey) -> Result<InputSourceHandle, InputError> {
        if self.0.sources.get(key).is_none() {
            return Err(InputError::configuration_error());
        }
        Ok(InputSourceHandle {
            key,
            admission: self.clone(),
        })
    }

    /// Submit caller-owned observations and progress. Wait until the coordinator admits them.
    /// Prepare the batch before this call. Request and reply channel storage is reused.
    pub fn submit(&self, batch: InputBatch) -> Result<InputReceipt, InputError> {
        self.request(AdmissionOperation::Submit(batch))
    }

    /// Publish an exclusive acquisition frontier without advancing the selected clock.
    pub fn advance(
        &self,
        source: &InputSourceHandle,
        frontier: PhysicalInstant,
    ) -> Result<(), InputError> {
        self.request(AdmissionOperation::Advance(source.clone(), frontier))
            .map(|_| ())
    }

    /// Disconnect a source under its required or optional error policy.
    pub fn disconnect(&self, source: &InputSourceHandle) -> InputError {
        self.request(AdmissionOperation::Disconnect(source.clone()))
            .unwrap_err()
    }

    fn request(&self, operation: AdmissionOperation) -> Result<InputReceipt, InputError> {
        let request = AdmissionRequest {
            arrival: Instant::now(),
            operation,
        };
        // The lease pairs a single outstanding request with its reusable reply channel.
        self.0.leases.recv().map_err(|_| self.closed_error())?;
        let lease = RequestLease(&self.0.lease_return);
        self.0
            .requests
            .send(request)
            .map_err(|_| self.closed_error())?;
        self.0.notify.input_ready();
        let response = self.0.replies.recv().map_err(|_| self.closed_error())?;
        drop(lease);
        response
    }

    fn closed_error(&self) -> InputError {
        self.0
            .failure
            .get()
            .cloned()
            .unwrap_or_else(|| InputError::new(InputErrorKind::Disconnected))
    }

    /// Close all wait paths before application state or workers are dropped.
    pub(crate) fn finish(&self) -> Result<(), InputError> {
        let _ = self.0.leases.close();
        let _ = self.0.requests.close();
        // The processor owns the only reply sender. Dropping it wakes a pending
        // caller while preserving a receipt that was already queued.
        self.0.notify.abort();
        self.0.failure.get().cloned().map_or(Ok(()), Err)
    }
}

/// Returns the reusable lane on normal return and unwinding.
struct RequestLease<'a>(&'a crate::Sender<()>);
impl Drop for RequestLease<'_> {
    fn drop(&mut self) {
        let _ = self.0.try_send(());
    }
}

impl Drop for AdmissionProcessor {
    fn drop(&mut self) {
        // Kanal clears queued requests under its lock. Their source handles can
        // own the last endpoint. Keep it alive until that lock has been released.
        let endpoint = self.engine.endpoint.upgrade();
        if let Some(error) = &self.state.failure {
            let _ = self.failure.set(error.clone());
        }
        let _ = self.leases.close();
        let _ = self.requests.close();
        drop(endpoint);
        // Let the sole reply sender drop naturally so an admitted receipt survives.
    }
}

impl AdmissionProcessor {
    /// Process at most one owned request. The report queue provides wakeup and ordering.
    pub(crate) fn process(&mut self, coordination: &mut FederateCoordinationState) {
        let Ok(Some(request)) = self.requests.try_recv() else {
            return;
        };
        let result = match request.operation {
            AdmissionOperation::Submit(batch) => {
                self.engine
                    .submit(&mut self.state, coordination, batch, request.arrival)
            }
            AdmissionOperation::Advance(source, frontier) => self.engine.submit_progress(
                &mut self.state,
                coordination,
                &[(source, frontier)],
                request.arrival,
            ),
            AdmissionOperation::Disconnect(source) => {
                Err(self
                    .engine
                    .disconnect(&mut self.state, coordination, &source, request.arrival))
            }
        };
        if let Some(error) = &self.state.failure {
            let _ = self.failure.set(error.clone());
        }
        let _ = self.replies.try_send(result);
    }
}

/// Get the minimum required-source limit. Unknown frontiers block all finite tags.
/// An empty required-source set leaves execution unbounded.
fn aggregate(frontiers: impl Iterator<Item = Option<Tag>>) -> Tag {
    frontiers
        .map(|frontier| frontier.unwrap_or(Tag::NEVER))
        .min()
        .unwrap_or(Tag::FOREVER)
}

impl AdmissionProcessor {
    /// Create the endpoint and owned processor before startup.
    pub(crate) fn new(
        config: InputConfig,
        delays: tinymap::TinyMap<SourceKey, Duration>,
        clock: impl Into<SelectedClock>,
        senders: tinymap::TinySecondaryMap<EnclaveIndex, crate::Sender<AsyncEvent>>,
        coordination: &mut FederateCoordinationState,
        abort: crate::sched::federate::FederateAbortHandle,
    ) -> (InputAdmission, Self) {
        let source_keys = config.sources.values().map(|_| ()).collect();
        let sources: tinymap::TinyMap<SourceKey, SourceState> = config
            .sources
            .into_iter()
            .map(|(key, source)| SourceState {
                id: source.id,
                actions: source.actions,
                required: source.required,
                enabled: true,
                last: None,
                frontier: None,
                delay: delays[key],
            })
            .collect();
        let horizon = aggregate(sources.values().filter(|s| s.required).map(|_| None));
        coordination.input_horizon = Some(horizon);
        let scratch = AdmissionScratch {
            sources: sources.values().map(|_| PendingSource::default()).collect(),
            reservations: Vec::with_capacity(config.max_batch_values),
        };
        let retained = RetainedBatches::new(config.max_batch_values, config.max_staged_batches);
        let clock = clock.into();
        let failure = Arc::new(OnceLock::new());
        let (request_tx, requests) = kanal::bounded(1);
        let (replies, reply_rx) = kanal::bounded(1);
        let (leases, lease_rx) = kanal::bounded(1);
        leases.send(()).expect("new lease channel is open");
        let endpoint = InputAdmission(Arc::new(AdmissionService {
            clock: clock.clone(),
            sources: source_keys,
            requests: request_tx,
            replies: reply_rx,
            leases: lease_rx,
            lease_return: leases.clone(),
            failure: failure.clone(),
            notify: abort.clone(),
        }));
        let weak_endpoint = Arc::downgrade(&endpoint.0);
        (
            endpoint,
            Self {
                engine: AdmissionEngine {
                    clock,
                    endpoint: weak_endpoint,
                    senders,
                    max_values: config.max_batch_values,
                    max_batches: config.max_staged_batches,
                    abort,
                },
                state: AdmissionState {
                    sources,
                    retained,
                    scratch,
                    horizon,
                    failure: None,
                },
                requests,
                replies,
                leases,
                failure,
            },
        )
    }
}

impl AdmissionEngine {
    fn reject(
        &self,
        state: &mut AdmissionState,
        source: Option<SourceKey>,
        kind: InputErrorKind,
        arrival: Instant,
    ) -> InputError {
        let source = source
            .filter(|key| state.sources.get(*key).is_some())
            .map(|key| &mut state.sources[key]);
        let diagnostic = InputError {
            source_id: source.as_ref().map(|s| s.id.clone()),
            kind,
            arrival,
        };
        if let Some(source) = source {
            if matches!(
                kind,
                InputErrorKind::Protocol | InputErrorKind::Disconnected
            ) {
                source.enabled = false;
            }
            if source.required
                && matches!(
                    kind,
                    InputErrorKind::Protocol
                        | InputErrorKind::Disconnected
                        | InputErrorKind::Overflow
                )
            {
                state.failure.get_or_insert_with(|| diagnostic.clone());
                self.abort.abort();
            }
        }
        diagnostic
    }
    /// Disconnect a source. Previously submitted optional observations remain available for execution.
    fn disconnect(
        &self,
        state: &mut AdmissionState,
        coordination: &mut FederateCoordinationState,
        source: &InputSourceHandle,
        arrival: Instant,
    ) -> InputError {
        if !Weak::ptr_eq(&Arc::downgrade(&source.admission.0), &self.endpoint)
            || state.sources.get(source.key).is_none()
        {
            return InputError::new(InputErrorKind::Malformed);
        }
        let diagnostic = self.reject(
            state,
            Some(source.key),
            InputErrorKind::Disconnected,
            arrival,
        );
        if state.failure.is_some() {
            coordination.input_horizon = Some(Tag::NEVER);
        }
        diagnostic
    }
    fn submit_progress(
        &self,
        state: &mut AdmissionState,
        coordination: &mut FederateCoordinationState,
        progress: &[(InputSourceHandle, PhysicalInstant)],
        arrival: Instant,
    ) -> Result<InputReceipt, InputError> {
        self.admit(
            state,
            coordination,
            InputBatch::new(Vec::new()),
            progress,
            arrival,
        )
    }

    fn submit(
        &self,
        state: &mut AdmissionState,
        coordination: &mut FederateCoordinationState,
        mut batch: InputBatch,
        arrival: Instant,
    ) -> Result<InputReceipt, InputError> {
        let progress = std::mem::take(&mut batch.progress);
        self.admit(state, coordination, batch, &progress, arrival)
    }

    /// Validate a prepared batch, then publish its values before its progress.
    ///
    /// Create the [`InputBatch`] before submission to allocate its delivery and receipt buffers.
    /// Admission reuses its validation and retention storage. Coordinator notifications and
    /// error diagnostics can still allocate. Partial publication aborts execution.
    /// Rejection can disable a source or retain an error without publishing values.
    fn admit(
        &self,
        state: &mut AdmissionState,
        coordination: &mut FederateCoordinationState,
        mut batch: InputBatch,
        progress: &[(InputSourceHandle, PhysicalInstant)],
        arrival: Instant,
    ) -> Result<InputReceipt, InputError> {
        if let Some(error) = &state.failure {
            return Err(error.clone());
        }
        if self.clock.now().is_err() {
            return Err(InputError::new(InputErrorKind::Disconnected));
        }
        if coordination.is_stopped() {
            return Err(InputError::new(InputErrorKind::Disconnected));
        }
        state.retained.reclaim(|reservation| {
            !coordination.input_completed(reservation.action.enclave, reservation.tag)
        });
        let now = self
            .clock
            .now()
            .map_err(|_| InputError::new(InputErrorKind::Disconnected))?;
        let required = self
            .validate_batch(state, coordination, &batch, progress, now)
            .map_err(|(source, kind)| {
                let diagnostic = self.reject(state, Some(source), kind, arrival);
                if state.failure.is_some() {
                    coordination.input_horizon = Some(Tag::NEVER);
                }
                diagnostic
            })?;

        batch.route_values(
            state
                .scratch
                .reservations
                .iter()
                .map(|reservation| reservation.tag),
        );
        let sent = self.publish_batch(state, coordination, &mut batch, required, arrival)?;
        let changed = self.commit_batch(state, coordination);
        if sent || changed {
            self.wake_schedulers();
        }
        Ok(batch.receipt(arrival))
    }

    /// Check all observations and progress without committing source state.
    fn validate_batch(
        &self,
        state: &mut AdmissionState,
        coordination: &FederateCoordinationState,
        batch: &InputBatch,
        progress: &[(InputSourceHandle, PhysicalInstant)],
        now: PhysicalInstant,
    ) -> Result<Option<SourceKey>, (SourceKey, InputErrorKind)> {
        state.scratch.reset(&state.sources);
        let required = batch.observations.iter().map(|o| o.source.key).find(|key| {
            state
                .sources
                .get(*key)
                .is_some_and(|source| source.required)
        });
        for observation in &batch.observations {
            self.validate_observation(state, coordination, observation, now)
                .map_err(|kind| {
                    let source = required
                        .filter(|_| kind == InputErrorKind::Overflow)
                        .unwrap_or(observation.source.key);
                    (source, kind)
                })?;
        }
        for (source, frontier) in progress {
            self.validate_progress(state, source, *frontier)
                .map_err(|kind| (source.key, kind))?;
        }
        Ok(required)
    }

    /// Validate one acquisition and reserve its action/tag pairs in scratch storage.
    fn validate_observation(
        &self,
        state: &mut AdmissionState,
        coordination: &FederateCoordinationState,
        observation: &InputObservation,
        now: PhysicalInstant,
    ) -> Result<(), InputErrorKind> {
        let key = observation.source.key;
        let source = state.sources.get(key).ok_or(InputErrorKind::Malformed)?;
        if !source.enabled {
            return Err(InputErrorKind::Disconnected);
        }
        if !Weak::ptr_eq(
            &Arc::downgrade(&observation.source.admission.0),
            &self.endpoint,
        ) || observation.values.is_empty()
            || observation.domain != self.clock.domain()
            || observation.epoch != self.clock.epoch()
        {
            return Err(InputErrorKind::Malformed);
        }
        if source.frontier.is_some_and(|f| observation.acquired < f) {
            return Err(InputErrorKind::Protocol);
        }
        if observation.acquired > now {
            return Err(InputErrorKind::Future);
        }
        if let Some((sequence, acquired)) = state.scratch.sources[key].last {
            if observation.sequence == sequence {
                return Err(InputErrorKind::Duplicate);
            }
            if observation.sequence < sequence || observation.acquired < acquired {
                return Err(InputErrorKind::OutOfOrder);
            }
        }
        for value in &observation.values {
            let target = &value.action;
            if target.source.key != key
                || !Weak::ptr_eq(&Arc::downgrade(&target.source.admission.0), &self.endpoint)
                || !source.actions.contains(&target.reference)
                || value.value.as_ref().type_id() != target.payload
            {
                return Err(InputErrorKind::Malformed);
            }
            let tag = observation
                .acquired
                .to_tag(target.delay)
                .map_err(|_| InputErrorKind::Malformed)?;
            if coordination.input_is_late(target.reference.enclave, tag) {
                return Err(InputErrorKind::Late);
            }
            if state
                .scratch
                .reservations
                .iter()
                .chain(state.retained.entries.iter())
                .any(|reservation| reservation.action == target.reference && reservation.tag == tag)
                || state.scratch.reservations.len() == self.max_values
                || state.retained.len() == self.max_batches
            {
                return Err(InputErrorKind::Overflow);
            }
            state.scratch.reservations.push(Reservation {
                action: target.reference,
                tag,
            });
        }
        state.scratch.sources[key].last = Some((observation.sequence, observation.acquired));
        Ok(())
    }

    /// Check one exclusive frontier against the tentative source state.
    fn validate_progress(
        &self,
        state: &mut AdmissionState,
        source_handle: &InputSourceHandle,
        frontier: PhysicalInstant,
    ) -> Result<(), InputErrorKind> {
        let key = source_handle.key;
        let source = state.sources.get(key).ok_or(InputErrorKind::Malformed)?;
        if !source.enabled {
            return Err(InputErrorKind::Disconnected);
        }
        if !Weak::ptr_eq(&Arc::downgrade(&source_handle.admission.0), &self.endpoint) {
            return Err(InputErrorKind::Malformed);
        }
        if state.scratch.sources[key]
            .frontier
            .is_some_and(|f| frontier < f)
        {
            return Err(InputErrorKind::Protocol);
        }
        frontier
            .to_tag(source.delay)
            .map_err(|_| InputErrorKind::Malformed)?;
        state.scratch.sources[key].frontier = Some(frontier);
        Ok(())
    }

    /// Transfer all envelopes before the coordinator can issue another execution permit.
    fn publish_batch(
        &self,
        state: &mut AdmissionState,
        coordination: &mut FederateCoordinationState,
        batch: &mut InputBatch,
        required: Option<SourceKey>,
        arrival: Instant,
    ) -> Result<bool, InputError> {
        let mut sent = false;
        for (enclave, envelope) in batch.envelopes.iter_mut() {
            let source = required.or_else(|| {
                state
                    .scratch
                    .reservations
                    .iter()
                    .find(|reservation| reservation.action.enclave == enclave)
                    .and_then(|reservation| {
                        state
                            .sources
                            .iter()
                            .find(|(_, source)| source.actions.contains(&reservation.action))
                            .map(|(key, _)| key)
                    })
            });
            let event = AsyncEvent::PhysicalBatch(super::InputBatchEvent {
                values: std::mem::take(&mut envelope.values),
            });
            match self.senders[enclave].try_send(event) {
                Ok(true) => {
                    if !sent {
                        coordination.input_publishing(
                            state
                                .scratch
                                .reservations
                                .iter()
                                .map(|reservation| reservation.action.enclave),
                        );
                    }
                    sent = true;
                }
                result => {
                    let kind = if result.is_err() {
                        InputErrorKind::Disconnected
                    } else {
                        InputErrorKind::Overflow
                    };
                    let diagnostic = self.reject(state, source, kind, arrival);
                    if sent || state.failure.is_some() {
                        state.failure.get_or_insert_with(|| diagnostic.clone());
                        coordination.input_horizon = Some(Tag::NEVER);
                        self.abort.abort();
                    }
                    self.wake_schedulers();
                    return Err(diagnostic);
                }
            }
        }
        Ok(sent)
    }

    /// Commit reservations and source updates after every envelope was accepted.
    fn commit_batch(
        &self,
        state: &mut AdmissionState,
        coordination: &mut FederateCoordinationState,
    ) -> bool {
        if !state.scratch.reservations.is_empty() {
            state.retained.append(&state.scratch.reservations);
        }
        let changed = state
            .sources
            .values()
            .zip(state.scratch.sources.values())
            .any(|(source, pending)| source.frontier != pending.frontier);
        for (source, pending) in state
            .sources
            .values_mut()
            .zip(state.scratch.sources.values())
        {
            source.last = pending.last;
            source.frontier = pending.frontier;
        }
        if changed {
            state.horizon = aggregate(
                state
                    .sources
                    .values()
                    .filter(|s| s.required)
                    .map(|s| s.frontier.map(|f| f.to_tag(s.delay).unwrap().decrement())),
            );
            coordination.input_horizon = Some(state.horizon);
        }
        changed
    }

    /// Wake publication or terminal processing without waiting for mailbox capacity.
    fn wake_schedulers(&self) {
        for tx in self.senders.values() {
            let _ = tx.try_send(AsyncEvent::FederateResume);
        }
    }
}

#[cfg(all(test, feature = "hosted-manual-clock"))]
#[path = "tests.rs"]
mod tests;
