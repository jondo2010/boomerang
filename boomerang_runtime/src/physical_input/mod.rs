//! Bounded hosted physical-input admission and exclusive source progress.
//!
//! # Setup and ownership
//!
//! [`crate::FederateBindings::with_physical_inputs`] installs an [`InputConfig`]
//! sidecar. Before initializers or drivers run, resolution validates declared
//! source and physical-action target identities, payload types, and minimum
//! delays. Targets belong exclusively to the adapter; other publishers must use
//! separate actions. The startup callback receives this run's [`InputAdmission`].
//! All admission state and grant checks are absent without `external-clock`.
//!
//! # Atomic admission and reservations
//!
//! Adapters decode payloads before submission and resolve source/target names once.
//! [`InputAdmission::submit`] validates an entire coherent batch, publishes at most
//! one envelope per destination Enclave, then commits exclusive source progress.
//! Receipts retain host arrival separately from mapped tags. Mapping uses checked
//! acquisition time plus the compiled minimum delay; diagnostic arrival time does
//! not affect scheduling.
//!
//! Reservations prevent replacement of a pending value at the same target/tag,
//! including across batches. Value and retained-batch bounds produce explicit
//! overflow. Retention is released when destination tags complete. The scheduler
//! preserves these already-admitted tags instead of normalizing them as raw
//! physical events. Partial fan-out aborts before a batch can execute.
//!
//! # Exclusive progress and authorization
//!
//! Advancing a source to F promises that later acquisition times are not less
//! than F, so an observation at F remains admissible. Required sources initially
//! gate all finite tags. The minimum predecessor of each required frontier plus
//! target delay constrains the existing Federate grant horizon. This aggregate is
//! cached on progress changes; generation-bound authorization fences execution
//! against concurrent admission. Optional sources never constrain the horizon.
//!
//! Required sources keep an idle Federate alive, while shutdown and configured
//! logical horizons continue through the existing coordinator and selected clock.
//! Frontiers may exceed clock time; observations may not. Required protocol
//! failures, overflow, and disconnection abort. Optional disconnection preserves
//! committed observations. [`InputErrorKind`] retains distinct rejection causes.
//! Clock advancement, admission, and progress remain separate responsibilities;
//! this module introduces no additional scheduler or coordinator.

use crate::{
    clock::ManualClock, clock::*, image::*, AsyncEvent, AsyncEventTarget, Duration, ReactorData,
    Tag,
};
use std::{
    any::TypeId,
    sync::{Arc, Mutex},
    time::Instant,
};

/// Dense source identity resolved once from a declared stable name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceKey(usize);
/// Dense target identity resolved once from a source's declared target name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TargetKey(usize);
/// A typed physical action declaration. Its delay comes from the compiled action image.
pub struct InputTarget {
    id: String,
    enclave: String,
    binding: String,
    payload: Option<TypeId>,
}
impl InputTarget {
    /// Declares a target whose payload type is resolved from its compiled action factory.
    pub fn from_binding(
        id: impl Into<String>,
        enclave: impl Into<String>,
        binding: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            enclave: enclave.into(),
            binding: binding.into(),
            payload: None,
        }
    }

    /// Declares an adapter-owned physical action using stable Enclave and payload-binding names.
    pub fn action<T: ReactorData>(
        id: impl Into<String>,
        enclave: impl Into<String>,
        binding: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            enclave: enclave.into(),
            binding: binding.into(),
            payload: Some(TypeId::of::<T>()),
        }
    }
}
/// One source's capabilities; periodic and idle sources publish explicit progress alike.
pub struct InputSource {
    /// Stable diagnostic and configuration identity.
    pub id: String,
    /// Required sources gate execution; protocol violations, overflow, or disconnection fail the Federate.
    pub required: bool,
    /// Declared typed destinations, each owned exclusively by this admission service.
    pub targets: Vec<InputTarget>,
}
/// Hosted Federate sidecar with explicit bounds on retained data.
pub struct InputConfig {
    /// Maximum total values in one coherent batch.
    pub max_batch_values: usize,
    /// Maximum batches retained until all their destination tags complete.
    pub max_staged_batches: usize,
    /// Sources resolved before driver or scheduler startup.
    pub sources: Vec<InputSource>,
}
/// An already decoded, uniquely owned payload; admission does not clone payload bytes.
pub struct InputValue {
    target: TargetKey,
    value: Box<dyn ReactorData>,
}
impl InputValue {
    /// Owns a decoded value for a declared target. Admission checks the concrete Rust type.
    pub fn new<T: ReactorData>(target: TargetKey, value: T) -> Self {
        Self {
            target,
            value: Box::new(value),
        }
    }
}
/// One acquisition, whose values are committed coherently with the rest of its batch.
pub struct InputObservation {
    /// Resolved declared source.
    pub source: SourceKey,
    /// Monotonically increasing source sequence.
    pub sequence: u64,
    /// Acquisition clock domain, never inferred from arrival time.
    pub domain: PhysicalClockDomainId,
    /// Fresh execution epoch supplied by the selected clock.
    pub epoch: ExecutionEpoch,
    /// Unsigned acquisition time within that epoch.
    pub acquired: PhysicalInstant,
    /// Owned decoded target values.
    pub values: Vec<InputValue>,
}
/// Successful coherent admission, retaining arrival separately from logical mapping.
#[derive(Debug)]
pub struct InputReceipt {
    /// Host-monotonic batch arrival, shared by every observation in this call.
    pub arrival: Instant,
    /// Mapped tags in observation/value order, independent of arrival time.
    pub tags: Vec<Tag>,
}
/// Explicit admission or source protocol outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputErrorKind {
    /// Invalid declaration, identity, payload, clock identity, or checked mapping.
    Malformed,
    /// Acquisition time exceeds the selected clock; retry is permitted.
    Future,
    /// A destination already completed or reserved the mapped tag for execution.
    Late,
    /// Sequence matches the accepted high-water mark or an earlier observation in this batch.
    Duplicate,
    /// Older sequence or acquisition time; no unbounded duplicate history is retained.
    OutOfOrder,
    /// Observation contradicts an exclusive frontier, or frontier regresses.
    Protocol,
    /// Configured staging, batch, mailbox, or same-target/tag capacity is exhausted.
    Overflow,
    /// Source, destination, clock, or execution is disconnected.
    Disconnected,
}
/// Diagnostic with stable identity and a separately captured host-monotonic arrival time.
#[derive(Clone, Debug, thiserror::Error)]
#[error("physical input {source_id:?}: {kind:?}")]
pub struct InputError {
    /// Stable source name, when the dense identity was valid.
    pub source_id: Option<String>,
    /// Closed error category.
    pub kind: InputErrorKind,
    /// Host-monotonic diagnostic timestamp; never used for logical mapping.
    pub arrival: Instant,
}
/// Creates a malformed-configuration diagnostic without a resolved source.
pub(crate) fn configuration_error() -> InputError {
    error(InputErrorKind::Malformed)
}
/// Creates a source-independent diagnostic with the current host timestamp.
fn error(kind: InputErrorKind) -> InputError {
    InputError {
        source_id: None,
        kind,
        arrival: Instant::now(),
    }
}
/// Resolved physical-action destination, payload type, source owner, and compiled delay.
pub(crate) struct Target {
    id: String,
    source: SourceKey,
    enclave: usize,
    action: crate::ActionKey,
    delay: Duration,
    payload: TypeId,
}
/// Per-source sequence, acquisition time, exclusive frontier, and minimum target delay.
struct Source {
    id: String,
    required: bool,
    enabled: bool,
    last: Option<(u64, PhysicalInstant)>,
    frontier: Option<PhysicalInstant>,
    delay: Duration,
}
/// Per-Enclave mailbox count, execution reservation, and generation-bound authorization.
struct Participant {
    tx: crate::Sender<AsyncEvent>,
    reserved: Tag,
    mailbox: usize,
    authorized: Option<(Tag, u64)>,
}
/// Synchronized admission, progress publication, and execution reservation state.
struct State {
    generation: u64,
    sources: Vec<Source>,
    participants: Vec<Participant>,
    retained: Vec<Vec<(usize, TargetKey, Tag)>>,
    horizon: Tag,
    failure: Option<InputError>,
    closed: bool,
}
/// Execution-scoped clock, resolved targets, bounds, and abort handle shared by admission.
struct Service {
    clock: ManualClock,
    targets: Vec<Target>,
    max_values: usize,
    max_batches: usize,
    state: Mutex<State>,
    abort: crate::sched::federate::FederateAbortHandle,
}
/// Resolved admission handle for exactly one execution; safe to clone into hosted drivers.
#[derive(Clone)]
pub struct InputAdmission(Arc<Service>);
/// One mailbox envelope per affected Enclave. Only admission can construct a valid envelope.
pub struct InputBatchEvent {
    pub(crate) values: Vec<(Tag, AsyncEventTarget, Box<dyn ReactorData>)>,
}
impl std::fmt::Debug for InputAdmission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InputAdmission").finish_non_exhaustive()
    }
}
impl std::fmt::Debug for InputBatchEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InputBatchEvent")
            .field("values", &self.values.len())
            .finish()
    }
}
/// Returns the minimum required-source cap; unknown frontiers block all finite tags.
/// An empty required-source set leaves execution unbounded.
fn aggregate(frontiers: impl Iterator<Item = Option<Tag>>) -> Tag {
    frontiers
        .map(|frontier| frontier.unwrap_or(Tag::NEVER))
        .min()
        .unwrap_or(Tag::FOREVER)
}
impl InputConfig {
    /// Validates bounded declarations and resolves stable names to typed compiled targets.
    pub(crate) fn resolve(
        &self,
        images: &tinymap::TinySecondaryMap<EnclaveIndex, EnclaveImageView<'_>>,
        bindings: &tinymap::TinySecondaryMap<EnclaveIndex, crate::storage::owned::EnclaveBindings>,
    ) -> Result<Vec<Target>, InputError> {
        if self.max_batch_values == 0
            || self.max_staged_batches == 0
            || self
                .max_batch_values
                .checked_mul(self.max_staged_batches)
                .is_none()
        {
            return Err(error(InputErrorKind::Malformed));
        }
        let mut targets = Vec::new();
        for (index, source) in self.sources.iter().enumerate() {
            if source.id.is_empty()
                || source.targets.is_empty()
                || self.sources[..index].iter().any(|s| s.id == source.id)
            {
                return Err(error(InputErrorKind::Malformed));
            }
            for target in &source.targets {
                let (slot, (enclave, image)) = images
                    .iter()
                    .enumerate()
                    .find(|(_, (_, image))| image.enclave_id().as_str() == target.enclave)
                    .ok_or_else(|| error(InputErrorKind::Malformed))?;
                let (binding, _) = image
                    .required_bindings()
                    .iter()
                    .find(|(_, binding)| binding.id().as_str() == target.binding)
                    .ok_or_else(|| error(InputErrorKind::Malformed))?;
                let action = image
                    .actions()
                    .values()
                    .find(|action| action.binding() == Some(binding))
                    .ok_or_else(|| error(InputErrorKind::Malformed))?;
                let ActionTiming::Standard {
                    domain: TimingDomain::Physical,
                    min_delay_nanos,
                } = action.timing()
                else {
                    return Err(error(InputErrorKind::Malformed));
                };
                let delay = Duration::nanoseconds(
                    i64::try_from(*min_delay_nanos)
                        .map_err(|_| error(InputErrorKind::Malformed))?,
                );
                let action = crate::ActionKey::from(action.storage_slot().as_u32() as usize);
                let payload = bindings[enclave]
                    .action_payload_type(binding)
                    .ok_or_else(|| error(InputErrorKind::Malformed))?;
                if target.payload.is_some_and(|expected| expected != payload)
                    || target.id.is_empty()
                    || targets.iter().any(|existing: &Target| {
                        (existing.enclave == slot && existing.action == action)
                            || (existing.source == SourceKey(index) && existing.id == target.id)
                    })
                {
                    return Err(error(InputErrorKind::Malformed));
                }
                targets.push(Target {
                    id: target.id.clone(),
                    source: SourceKey(index),
                    enclave: slot,
                    action,
                    delay,
                    payload,
                });
            }
        }
        Ok(targets)
    }
}
impl InputAdmission {
    /// Allocates bounded source and participant state before starting the input driver.
    pub(crate) fn new(
        config: InputConfig,
        targets: Vec<Target>,
        clock: ManualClock,
        senders: &[crate::Sender<AsyncEvent>],
        abort: crate::sched::federate::FederateAbortHandle,
    ) -> Self {
        let sources: Vec<_> = config
            .sources
            .into_iter()
            .enumerate()
            .map(|(index, source)| Source {
                id: source.id,
                required: source.required,
                enabled: true,
                last: None,
                frontier: None,
                delay: targets
                    .iter()
                    .filter(|t| t.source == SourceKey(index))
                    .map(|t| t.delay)
                    .min()
                    .unwrap(),
            })
            .collect();
        let horizon = aggregate(sources.iter().filter(|s| s.required).map(|_| None));
        Self(Arc::new(Service {
            clock,
            targets,
            max_values: config.max_batch_values,
            max_batches: config.max_staged_batches,
            state: Mutex::new(State {
                generation: 0,
                sources,
                participants: senders
                    .iter()
                    .map(|tx| Participant {
                        tx: tx.clone(),
                        reserved: Tag::NEVER,
                        mailbox: 0,
                        authorized: None,
                    })
                    .collect(),
                retained: Vec::with_capacity(config.max_staged_batches),
                horizon,
                failure: None,
                closed: false,
            }),
            abort,
        }))
    }
    /// Resolves a declared stable source identity once at driver startup.
    pub fn source(&self, id: &str) -> Option<SourceKey> {
        self.0
            .state
            .lock()
            .unwrap()
            .sources
            .iter()
            .position(|s| s.id == id)
            .map(SourceKey)
    }
    /// Resolves one of a source's declared target names once at driver startup.
    pub fn target(&self, source: SourceKey, id: &str) -> Option<TargetKey> {
        self.0
            .targets
            .iter()
            .position(|t| t.source == source && t.id == id)
            .map(TargetKey)
    }
    /// Builds a diagnostic and applies source disabling or terminal required-source failure.
    fn reject(
        &self,
        state: &mut State,
        source: Option<SourceKey>,
        kind: InputErrorKind,
        arrival: Instant,
    ) -> InputError {
        let source = source.and_then(|key| state.sources.get_mut(key.0));
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
    /// Disconnects a source. Committed optional observations remain retained and executable.
    pub fn disconnect(&self, source: SourceKey) -> InputError {
        let mut state = self.0.state.lock().unwrap();
        if source.0 >= state.sources.len() {
            return error(InputErrorKind::Malformed);
        }
        self.reject(
            &mut state,
            Some(source),
            InputErrorKind::Disconnected,
            Instant::now(),
        )
    }
    /// Publishes an exclusive acquisition frontier, including explicit idle progress.
    /// Future acquisitions must be greater than or equal to the frontier. Progress
    /// may exceed the current clock time; it does not advance that clock.
    pub fn advance(&self, source: SourceKey, frontier: PhysicalInstant) -> Result<(), InputError> {
        self.submit(Vec::new(), &[(source, frontier)]).map(|_| ())
    }
    /// Validates the whole batch before fan-out, then publishes values before progress.
    /// Returns mapped tags and host diagnostic arrival. Partial publication aborts
    /// execution; rejection may disable a source or latch failure without committing values.
    pub fn submit(
        &self,
        observations: Vec<InputObservation>,
        progress: &[(SourceKey, PhysicalInstant)],
    ) -> Result<InputReceipt, InputError> {
        let arrival = Instant::now();
        let mut state = self.0.state.lock().unwrap();
        if let Some(error) = &state.failure {
            return Err(error.clone());
        }
        if state.closed || self.0.clock.now().is_err() {
            return Err(error(InputErrorKind::Disconnected));
        }
        let now = self
            .0
            .clock
            .now()
            .map_err(|_| error(InputErrorKind::Disconnected))?;
        let mut last: Vec<_> = state.sources.iter().map(|s| s.last).collect();
        let mut frontiers: Vec<_> = state.sources.iter().map(|s| s.frontier).collect();
        let mut reservations = Vec::new();
        let mut mapped = Vec::new();
        let required = observations.iter().map(|o| o.source).find(|key| {
            state
                .sources
                .get(key.0)
                .is_some_and(|source| source.required)
        });
        for observation in &observations {
            let key = observation.source;
            let validation = (|| {
                let source = state.sources.get(key.0).ok_or(InputErrorKind::Malformed)?;
                if !source.enabled {
                    return Err(InputErrorKind::Disconnected);
                }
                if observation.values.is_empty()
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
                if let Some((sequence, acquired)) = last[key.0] {
                    if observation.sequence == sequence {
                        return Err(InputErrorKind::Duplicate);
                    }
                    if observation.sequence < sequence || observation.acquired < acquired {
                        return Err(InputErrorKind::OutOfOrder);
                    }
                }
                for value in &observation.values {
                    let target = self
                        .0
                        .targets
                        .get(value.target.0)
                        .ok_or(InputErrorKind::Malformed)?;
                    if target.source != key || value.value.as_ref().type_id() != target.payload {
                        return Err(InputErrorKind::Malformed);
                    }
                    let tag = observation
                        .acquired
                        .to_tag(target.delay)
                        .map_err(|_| InputErrorKind::Malformed)?;
                    if tag <= state.participants[target.enclave].reserved {
                        return Err(InputErrorKind::Late);
                    }
                    if reservations
                        .iter()
                        .chain(state.retained.iter().flatten())
                        .any(|&(_, t, pending)| t == value.target && pending == tag)
                    {
                        return Err(InputErrorKind::Overflow);
                    }
                    if reservations.len() == self.0.max_values
                        || state.retained.len() == self.0.max_batches
                    {
                        return Err(InputErrorKind::Overflow);
                    }
                    reservations.push((target.enclave, value.target, tag));
                    mapped.push(tag);
                }
                last[key.0] = Some((observation.sequence, observation.acquired));
                Ok(())
            })();
            if let Err(kind) = validation {
                let key = required
                    .filter(|_| kind == InputErrorKind::Overflow)
                    .unwrap_or(key);
                return Err(self.reject(&mut state, Some(key), kind, arrival));
            }
        }
        for &(key, frontier) in progress {
            let validation = (|| {
                let source = state.sources.get(key.0).ok_or(InputErrorKind::Malformed)?;
                if !source.enabled {
                    return Err(InputErrorKind::Disconnected);
                }
                if frontiers[key.0].is_some_and(|f| frontier < f) {
                    return Err(InputErrorKind::Protocol);
                }
                frontier
                    .to_tag(source.delay)
                    .map_err(|_| InputErrorKind::Malformed)?;
                frontiers[key.0] = Some(frontier);
                Ok(())
            })();
            if let Err(kind) = validation {
                return Err(self.reject(&mut state, Some(key), kind, arrival));
            }
        }
        let mut envelopes: Vec<Vec<_>> =
            (0..state.participants.len()).map(|_| Vec::new()).collect();
        let mut tags = mapped.iter();
        for observation in observations {
            for value in observation.values {
                let target = &self.0.targets[value.target.0];
                envelopes[target.enclave].push((
                    *tags.next().unwrap(),
                    AsyncEventTarget::Action(target.action),
                    value.value,
                ));
            }
        }
        // Invalidate already queued grants before publishing any envelope.
        if !reservations.is_empty() {
            state.generation = state
                .generation
                .checked_add(1)
                .ok_or_else(configuration_error)?;
        }
        // Holding this lock serializes the complete fan-out against final execution reservations.
        let mut sent = false;
        for (slot, values) in envelopes
            .into_iter()
            .enumerate()
            .filter(|(_, values)| !values.is_empty())
        {
            // A rejected atomic batch loses every value, including required sources elsewhere.
            let source = required.or_else(|| {
                reservations
                    .iter()
                    .find(|(enclave, _, _)| *enclave == slot)
                    .map(|(_, target, _)| self.0.targets[target.0].source)
            });
            match state.participants[slot]
                .tx
                .try_send(AsyncEvent::PhysicalBatch(InputBatchEvent { values }))
            {
                Ok(true) => {
                    state.participants[slot].mailbox += 1;
                    sent = true;
                }
                result => {
                    let kind = if result.is_err() {
                        InputErrorKind::Disconnected
                    } else {
                        InputErrorKind::Overflow
                    };
                    let diagnostic = self.reject(&mut state, source, kind, arrival);
                    if sent {
                        state.failure.get_or_insert_with(|| diagnostic.clone());
                        self.0.abort.abort();
                    } else {
                        // Nothing was published; retain existing grants under this same lock.
                        state.generation -= 1;
                    }
                    return Err(diagnostic);
                }
            }
        }
        if !reservations.is_empty() {
            state.retained.push(reservations);
        }
        let changed = state
            .sources
            .iter()
            .zip(&frontiers)
            .any(|(source, frontier)| source.frontier != *frontier);
        for (index, source) in state.sources.iter_mut().enumerate() {
            source.last = last[index];
            source.frontier = frontiers[index];
        }
        if changed {
            state.horizon = aggregate(
                state
                    .sources
                    .iter()
                    .filter(|s| s.required)
                    .map(|s| s.frontier.map(|f| f.to_tag(s.delay).unwrap().decrement())),
            );
            self.0.abort.input_progress();
        }
        if sent || changed {
            for participant in &state.participants {
                let _ = participant.tx.try_send(AsyncEvent::FederateResume);
            }
        }
        Ok(InputReceipt {
            arrival,
            tags: mapped,
        })
    }
    /// Reads the cached input horizon and its generation together for grant acquisition.
    pub(crate) fn grant_constraint(&self) -> (Tag, u64) {
        let state = self.0.state.lock().unwrap();
        (
            if state.failure.is_some() || state.closed {
                Tag::NEVER
            } else {
                state.horizon
            },
            state.generation,
        )
    }
    /// Authorizes one participant only if the acquired input generation is still current.
    pub(crate) fn authorize(&self, slot: usize, tag: Tag, generation: u64) {
        let mut state = self.0.state.lock().unwrap();
        if generation == state.generation {
            state.participants[slot].authorized = Some((tag, generation));
        }
    }
    /// Reads the cached horizon for tests, blocking execution after closure or failure.
    #[cfg(test)]
    pub(crate) fn horizon(&self) -> Tag {
        let state = self.0.state.lock().unwrap();
        if state.failure.is_some() || state.closed {
            Tag::NEVER
        } else {
            state.horizon
        }
    }
    /// Acknowledges one consumed batch envelope without releasing retained target values.
    pub(crate) fn received(&self, slot: usize) {
        self.0.state.lock().unwrap().participants[slot].mailbox -= 1;
    }
    /// Atomically reserves an authorized tag after pending input envelopes are consumed.
    /// The cached horizon check is constant-time and fences concurrent late admission.
    pub(crate) fn reserve(&self, slot: usize, tag: Tag) -> bool {
        let mut state = self.0.state.lock().unwrap();
        if state.failure.is_some()
            || state.closed
            || tag > state.horizon
            || state.participants[slot].mailbox != 0
            || !state.participants[slot]
                .authorized
                .is_some_and(|(horizon, generation)| {
                    generation == state.generation && tag <= horizon
                })
        {
            return false;
        }
        state.participants[slot].reserved = tag;
        true
    }
    /// Releases retained destinations completed by this participant through the given tag.
    pub(crate) fn complete(&self, slot: usize, tag: Tag) {
        let mut state = self.0.state.lock().unwrap();
        for batch in &mut state.retained {
            batch.retain(|&(enclave, _, pending)| enclave != slot || pending > tag);
        }
        state.retained.retain(|batch| !batch.is_empty());
    }
    /// Closes admission at run exit and returns the first retained terminal input error.
    pub(crate) fn finish(&self) -> Result<(), InputError> {
        let mut state = self.0.state.lock().unwrap();
        state.closed = true;
        state.failure.clone().map_or(Ok(()), Err)
    }
}

#[cfg(test)]
mod tests;

/// Closes the execution admission handle when its run scope exits.
pub(crate) struct InputRun(pub(crate) Option<InputAdmission>);
impl Drop for InputRun {
    fn drop(&mut self) {
        if let Some(inputs) = &self.0 {
            let _ = inputs.finish();
        }
    }
}

/// Input declarations paired with the startup callback receiving resolved admission.
pub(crate) type InputSetup<'a> = (
    InputConfig,
    Box<dyn FnOnce(InputAdmission) -> Result<(), InputError> + Send + 'a>,
);
