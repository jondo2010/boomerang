//! Source protocol validation and batch publication.
//!
//! Admission locks source state before coordinator state. Coordinator paths do not
//! acquire the source lock. Both locks remain held through validation and publication.
//! The coordinator owns execution authority and publication fences.
//! A partially published batch stops execution before admission requests an abort.
//! Required-source protocol errors, overflow, and disconnection also abort execution.

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
    sync::{Arc, Mutex},
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
    closed: bool,
}
/// The shared clock, coordinator, source state, and scheduler mailboxes.
pub(super) struct AdmissionService {
    pub(super) clock: SelectedClock,
    coordination: Arc<Mutex<FederateCoordinationState>>,
    senders: tinymap::TinySecondaryMap<EnclaveIndex, crate::Sender<AsyncEvent>>,
    max_values: usize,
    max_batches: usize,
    state: Mutex<AdmissionState>,
    abort: crate::sched::federate::FederateAbortHandle,
}
/// An admission handle for one execution. You can clone it into application workers.
/// All clones share the same source state and execution identity.
#[derive(Clone)]
pub struct InputAdmission(pub(super) Arc<AdmissionService>);

impl std::fmt::Debug for InputAdmission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InputAdmission").finish_non_exhaustive()
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

impl InputAdmission {
    /// Create source protocol state and share the coordinator before startup.
    pub(crate) fn new(
        config: InputConfig,
        delays: tinymap::TinyMap<SourceKey, Duration>,
        clock: impl Into<SelectedClock>,
        senders: tinymap::TinySecondaryMap<EnclaveIndex, crate::Sender<AsyncEvent>>,
        coordination: Arc<Mutex<FederateCoordinationState>>,
        abort: crate::sched::federate::FederateAbortHandle,
    ) -> Self {
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
        coordination.lock().unwrap().input_horizon = Some(horizon);
        let scratch = AdmissionScratch {
            sources: sources.values().map(|_| PendingSource::default()).collect(),
            reservations: Vec::with_capacity(config.max_batch_values),
        };
        let retained = RetainedBatches::new(config.max_batch_values, config.max_staged_batches);
        Self(Arc::new(AdmissionService {
            clock: clock.into(),
            coordination,
            senders,
            max_values: config.max_batch_values,
            max_batches: config.max_staged_batches,
            state: Mutex::new(AdmissionState {
                sources,
                retained,
                scratch,
                horizon,
                failure: None,
                closed: false,
            }),
            abort,
        }))
    }
    /// Bind a compiled source index to this admission service.
    pub(crate) fn source_handle(&self, key: SourceKey) -> Result<InputSourceHandle, InputError> {
        if self.0.state.lock().unwrap().sources.get(key).is_none() {
            return Err(InputError::configuration_error());
        }
        Ok(InputSourceHandle {
            key,
            admission: self.clone(),
        })
    }
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
                self.0.abort.abort();
            }
        }
        diagnostic
    }
    /// Disconnect a source. Previously submitted optional observations remain available for execution.
    pub fn disconnect(&self, source: &InputSourceHandle) -> InputError {
        let mut state = self.0.state.lock().unwrap();
        if !Arc::ptr_eq(&source.admission.0, &self.0) || state.sources.get(source.key).is_none() {
            return InputError::new(InputErrorKind::Malformed);
        }
        let diagnostic = self.reject(
            &mut state,
            Some(source.key),
            InputErrorKind::Disconnected,
            Instant::now(),
        );
        if state.failure.is_some() {
            self.0.coordination.lock().unwrap().input_horizon = Some(Tag::NEVER);
        }
        diagnostic
    }
    /// Publish an exclusive acquisition frontier, including progress from an idle source.
    /// Future acquisitions must be greater than or equal to the frontier. Progress
    /// may exceed the current clock time; it does not advance that clock.
    pub fn advance(
        &self,
        source: &InputSourceHandle,
        frontier: PhysicalInstant,
    ) -> Result<(), InputError> {
        self.submit(InputBatch::new(Vec::new()), &[(source.clone(), frontier)])
            .map(|_| ())
    }
    /// Validate a prepared batch, then publish its values before its progress.
    ///
    /// Create the [`InputBatch`] before submission to allocate its delivery and receipt buffers.
    /// Admission reuses its validation and retention storage. Coordinator notifications and
    /// error diagnostics can still allocate. Partial publication aborts execution.
    /// Rejection can disable a source or retain an error without publishing values.
    pub fn submit(
        &self,
        mut batch: InputBatch,
        progress: &[(InputSourceHandle, PhysicalInstant)],
    ) -> Result<InputReceipt, InputError> {
        let arrival = Instant::now();
        let mut state = self.0.state.lock().unwrap();
        if let Some(error) = &state.failure {
            return Err(error.clone());
        }
        if state.closed || self.0.clock.now().is_err() {
            return Err(InputError::new(InputErrorKind::Disconnected));
        }
        let mut coordination = self.0.coordination.lock().unwrap();
        if coordination.is_stopped() {
            return Err(InputError::new(InputErrorKind::Disconnected));
        }
        state.retained.reclaim(|reservation| {
            !coordination.input_completed(reservation.action.enclave, reservation.tag)
        });
        let now = self
            .0
            .clock
            .now()
            .map_err(|_| InputError::new(InputErrorKind::Disconnected))?;
        let required = self
            .validate_batch(&mut state, &coordination, &batch, progress, now)
            .map_err(|(source, kind)| {
                let diagnostic = self.reject(&mut state, Some(source), kind, arrival);
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
        let sent =
            self.publish_batch(&mut state, &mut coordination, &mut batch, required, arrival)?;
        let changed = self.commit_batch(&mut state, &mut coordination);
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
        if !Arc::ptr_eq(&observation.source.admission.0, &self.0)
            || observation.values.is_empty()
            || observation.domain != self.0.clock.domain()
            || observation.epoch != self.0.clock.epoch()
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
                || !Arc::ptr_eq(&target.source.admission.0, &self.0)
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
                || state.scratch.reservations.len() == self.0.max_values
                || state.retained.len() == self.0.max_batches
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
        if !Arc::ptr_eq(&source_handle.admission.0, &self.0) {
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

    /// Transfer prepared envelopes while both admission locks remain held.
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
            match self.0.senders[enclave].try_send(event) {
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
                        self.0.abort.abort();
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
            self.0.abort.input_progress();
        }
        changed
    }

    /// Wake publication or terminal processing without waiting for mailbox capacity.
    fn wake_schedulers(&self) {
        for tx in self.0.senders.values() {
            let _ = tx.try_send(AsyncEvent::FederateResume);
        }
    }
    /// Get the cached execution limit for tests.
    #[cfg(all(test, feature = "hosted-manual-clock"))]
    fn horizon(&self) -> Tag {
        let state = self.0.state.lock().unwrap();
        if state.closed || state.failure.is_some() {
            Tag::NEVER
        } else {
            state.horizon
        }
    }
    /// Close admission at run exit and return the first retained terminal input error.
    pub(crate) fn finish(&self) -> Result<(), InputError> {
        let mut state = self.0.state.lock().unwrap();
        state.closed = true;
        self.0.coordination.lock().unwrap().input_horizon = Some(Tag::NEVER);
        state.failure.clone().map_or(Ok(()), Err)
    }
}

#[cfg(all(test, feature = "hosted-manual-clock"))]
#[path = "tests.rs"]
mod tests;
