//! Source protocol validation and batch publication.
//!
//! Admission locks source state before coordinator state. Coordinator paths do not
//! acquire the source lock. Both locks remain held through validation and publication.
//! The coordinator owns execution authority and publication fences.
//! A partially published batch stops execution before admission requests an abort.
//! Required-source protocol errors, overflow, and disconnection also abort execution.

use super::{
    retention::{Reservation, RetainedBatches},
    CompiledPhysicalAction, InputBatchEvent, InputConfig, InputError, InputErrorKind,
    InputObservation, InputReceipt, InputSourceHandle, SourceKey,
};
use crate::{
    clock::{PhysicalInstant, SelectedClock},
    image::EnclaveIndex,
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

/// Source protocol state and retained-batch counts. The coordinator owns execution authority.
struct AdmissionState {
    sources: tinymap::TinyMap<SourceKey, SourceState>,
    retained: RetainedBatches,
    horizon: Tag,
    failure: Option<InputError>,
    closed: bool,
}
/// The shared clock, coordinator, source state, and scheduler mailboxes.
pub(super) struct AdmissionService {
    pub(super) clock: SelectedClock,
    coordination: Arc<Mutex<crate::sched::federate::state::FederateCoordinationState>>,
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
        coordination: Arc<Mutex<crate::sched::federate::state::FederateCoordinationState>>,
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
        Self(Arc::new(AdmissionService {
            clock: clock.into(),
            coordination,
            senders,
            max_values: config.max_batch_values,
            max_batches: config.max_staged_batches,
            state: Mutex::new(AdmissionState {
                sources,
                retained: RetainedBatches::new(),
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
        self.submit(Vec::new(), &[(source.clone(), frontier)])
            .map(|_| ())
    }
    /// Validate the complete batch, then publish its values before its progress.
    /// Return the mapped tags and host arrival time. Partial publication aborts execution.
    /// Rejection can disable a source or retain an error without publishing values.
    pub fn submit(
        &self,
        observations: Vec<InputObservation>,
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
        let mut last: tinymap::TinyMap<SourceKey, _> =
            state.sources.values().map(|s| s.last).collect();
        let mut frontiers: tinymap::TinyMap<SourceKey, _> =
            state.sources.values().map(|s| s.frontier).collect();
        let mut reservations = Vec::new();
        let mut mapped = Vec::new();
        let required = observations.iter().map(|o| o.source.key).find(|key| {
            state
                .sources
                .get(*key)
                .is_some_and(|source| source.required)
        });
        for observation in &observations {
            let key = observation.source.key;
            let validation = (|| {
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
                if let Some((sequence, acquired)) = last[key] {
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
                    if reservations
                        .iter()
                        .chain(state.retained.entries.iter())
                        .any(|reservation: &Reservation| {
                            reservation.action == target.reference && reservation.tag == tag
                        })
                    {
                        return Err(InputErrorKind::Overflow);
                    }
                    if reservations.len() == self.0.max_values
                        || state.retained.len() == self.0.max_batches
                    {
                        return Err(InputErrorKind::Overflow);
                    }
                    reservations.push(Reservation {
                        action: target.reference,
                        tag,
                    });
                    mapped.push(tag);
                }
                last[key] = Some((observation.sequence, observation.acquired));
                Ok(())
            })();
            if let Err(kind) = validation {
                let key = required
                    .filter(|_| kind == InputErrorKind::Overflow)
                    .unwrap_or(key);
                let diagnostic = self.reject(&mut state, Some(key), kind, arrival);
                if state.failure.is_some() {
                    coordination.input_horizon = Some(Tag::NEVER);
                }
                return Err(diagnostic);
            }
        }
        for (source_handle, frontier) in progress {
            let key = source_handle.key;
            let frontier = *frontier;
            let validation = (|| {
                let source = state.sources.get(key).ok_or(InputErrorKind::Malformed)?;
                if !source.enabled {
                    return Err(InputErrorKind::Disconnected);
                }
                if !Arc::ptr_eq(&source_handle.admission.0, &self.0) {
                    return Err(InputErrorKind::Malformed);
                }
                if frontiers[key].is_some_and(|f| frontier < f) {
                    return Err(InputErrorKind::Protocol);
                }
                frontier
                    .to_tag(source.delay)
                    .map_err(|_| InputErrorKind::Malformed)?;
                frontiers[key] = Some(frontier);
                Ok(())
            })();
            if let Err(kind) = validation {
                let diagnostic = self.reject(&mut state, Some(key), kind, arrival);
                if state.failure.is_some() {
                    coordination.input_horizon = Some(Tag::NEVER);
                }
                return Err(diagnostic);
            }
        }
        let mut envelopes = tinymap::TinySecondaryMap::<EnclaveIndex, Vec<_>>::new();
        let mut tags = mapped.iter();
        for observation in observations {
            for value in observation.values {
                let enclave = value.action.reference.enclave;
                if !envelopes.contains_key(enclave) {
                    envelopes.insert(enclave, Vec::new());
                }
                envelopes[enclave].push((
                    *tags.next().unwrap(),
                    value.action.reference.action,
                    value.value,
                ));
            }
        }
        let mut sent = false;
        for (enclave, values) in envelopes {
            let source = required.or_else(|| {
                reservations
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
            match self.0.senders[enclave]
                .try_send(AsyncEvent::PhysicalBatch(InputBatchEvent { values }))
            {
                Ok(true) => {
                    if !sent {
                        coordination.input_publishing(
                            reservations
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
                    let diagnostic = self.reject(&mut state, source, kind, arrival);
                    if sent || state.failure.is_some() {
                        state.failure.get_or_insert_with(|| diagnostic.clone());
                        coordination.input_horizon = Some(Tag::NEVER);
                        self.0.abort.abort();
                    }
                    // Wake existing publication or terminal paths after a rejected fanout.
                    for tx in self.0.senders.values() {
                        let _ = tx.try_send(AsyncEvent::FederateResume);
                    }
                    return Err(diagnostic);
                }
            }
        }
        if !reservations.is_empty() {
            state.retained.append(&reservations);
        }
        let changed = state
            .sources
            .values()
            .zip(frontiers.values())
            .any(|(source, frontier)| source.frontier != *frontier);
        for ((source, last), frontier) in state
            .sources
            .values_mut()
            .zip(last.values())
            .zip(frontiers.values())
        {
            source.last = *last;
            source.frontier = *frontier;
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
        if sent || changed {
            for tx in self.0.senders.values() {
                let _ = tx.try_send(AsyncEvent::FederateResume);
            }
        }
        Ok(InputReceipt {
            arrival,
            tags: mapped,
        })
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
