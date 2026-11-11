//! Bounded checkpoint state owned by the existing scheduler authority.
//!
//! This module stores requests and reports proofs. It does not establish input
//! admissibility, perform scheduling, or close all microsteps at a logical time.
use crate::output::{
    Lifecycle, OutputBound, OutputCompletion, OutputEndpoint, OutputObserver, OutputOutcome,
    OutputRequest, OutputTarget, OutputWaitError,
};
use crate::{
    clock::ExecutionEpoch,
    image::{EnclaveIndex, ReactorIndex},
    Tag,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

/// Scheduler-owned requests, completed-tag proofs, and actual effect outcomes.
///
/// Declare this field before application state fields. Rust drops fields in
/// declaration order. Its drop must publish termination before an application
/// worker's drop can join a thread that waits for an output proof.
///
/// Setup allocates the owner table and reserves the pending-list capacity.
/// Ordinary reaction reporting and checkpoint completion use those allocations.
/// Host request creation has its own explicit allocations.
#[derive(Debug)]
pub(crate) struct OutputState {
    /// Execution-bound route, configured once and copied into host observers.
    endpoint: Option<OutputEndpoint>,
    /// Sticky export flag. Observer creation takes `&self`, so this is atomic.
    /// No payload is published through this flag; relaxed ordering is sufficient.
    /// Clock selection takes `&mut self` and cannot race observer creation.
    exported: AtomicBool,
    /// Native enclave identity used to validate every reactor target.
    enclave: Option<EnclaveIndex>,
    /// Unresolved requests. Shared lifecycle accounting bounds all retained stages,
    /// so this preallocated list never needs to grow during normal execution.
    pending: Vec<OutputRequest>,
    /// Highest bound for which the scheduler has already established a proof.
    completed: Option<OutputBound>,
    /// Sticky callback-unwind failure, separate from observer-only terminal errors.
    /// A retained scheduler cannot resume the interrupted tag, even if observation
    /// was already disabled for another reason.
    execution_interrupted: bool,
    /// Durable admission and computational frontiers; neither is an executed tag.
    pub(crate) progress: super::closure::progress::EnclaveOutputProgress,
    /// Bounded proof participation owned by this scheduler, not a second executor.
    pub(crate) proof: super::closure::protocol::ProofState,
    /// Latest actual outcome per reactor, including a completed prefix on failure.
    /// The core records these reports before it handles a later execution error.
    /// Keep known effects after failure; a checkpoint never invents an invocation.
    /// This is bounded evidence, not a per-invocation ledger. In particular, the
    /// future ledger for unresolved `Pending` outcomes is not implemented here.
    outcomes: tinymap::TinyMap<ReactorIndex, Option<(Tag, OutputOutcome)>>,
}
impl OutputState {
    /// Allocates one outcome slot for each owner supplied by validated topology.
    /// The state cannot export observers until setup binds its execution route.
    pub(crate) fn new(owners: impl Iterator<Item = ReactorIndex>) -> Self {
        let mut outcomes = tinymap::TinyMap::new();
        for owner in owners {
            assert_eq!(
                outcomes.insert(None),
                owner,
                "output owners must follow image order"
            );
        }
        Self {
            endpoint: None,
            exported: AtomicBool::new(false),
            enclave: None,
            pending: Vec::new(),
            completed: None,
            execution_interrupted: false,
            progress: Default::default(),
            proof: Default::default(),
            outcomes,
        }
    }

    /// Binds execution identity and reserves the existing mailbox capacity.
    ///
    /// # Errors
    ///
    /// Returns [`OutputWaitError::StaleExecution`] if setup already bound a route.
    pub(crate) fn configure(
        &mut self,
        enclave: EnclaveIndex,
        epoch: ExecutionEpoch,
        event_tx: crate::Sender<crate::AsyncEvent>,
    ) -> Result<(), OutputWaitError> {
        if self.endpoint.is_some() {
            return Err(OutputWaitError::StaleExecution);
        }
        let capacity = event_tx.capacity();
        self.pending.reserve_exact(capacity);
        self.enclave = Some(enclave);
        self.endpoint = Some(OutputEndpoint {
            epoch,
            capacity,
            event_tx,
            lifecycle: Arc::new(Lifecycle::default()),
        });
        Ok(())
    }

    /// Selects the clock epoch before any observer has left this state.
    /// An export permanently fixes the epoch, even if all observers are dropped.
    ///
    /// # Errors
    ///
    /// Returns [`crate::clock::PhysicalClockError::AlreadyUsed`] after an export.
    /// Host clock selection can also return its clock initialization error.
    #[cfg(feature = "hosted-manual-clock")]
    pub(crate) fn select_clock(
        &mut self,
        clock: &crate::clock::ExecutionClock,
    ) -> Result<(), crate::clock::PhysicalClockError> {
        let Some(endpoint) = self.endpoint.as_mut() else {
            return Ok(());
        };
        if self.exported.load(Ordering::Relaxed) {
            return Err(crate::clock::PhysicalClockError::AlreadyUsed);
        }
        let epoch = match clock {
            crate::clock::ExecutionClock::Manual { clock, .. } => clock.epoch(),
            crate::clock::ExecutionClock::HostMonotonic(_) => {
                crate::clock::SelectedClock::host_monotonic(clock.clone())?.epoch()
            }
        };
        endpoint.epoch = epoch;
        Ok(())
    }

    /// Exports the exact owner capability and permanently fixes its epoch.
    ///
    /// # Errors
    ///
    /// Returns [`OutputWaitError::InvalidTarget`] for an unknown or mismatched
    /// reactor, [`OutputWaitError::NotStarted`] before setup, or the execution's
    /// first terminal error after this owner terminates.
    pub(crate) fn observer(
        &self,
        reactor: ReactorIndex,
    ) -> Result<OutputObserver, OutputWaitError> {
        if self.outcomes.get(reactor).is_none() {
            return Err(OutputWaitError::InvalidTarget);
        }
        let endpoint = self.endpoint.as_ref().ok_or(OutputWaitError::NotStarted)?;
        if let Some(error) = endpoint.lifecycle.error() {
            return Err(error);
        }
        self.exported.store(true, Ordering::Relaxed);
        Ok(OutputObserver {
            endpoint: endpoint.clone(),
            target: OutputTarget {
                enclave: self.enclave.ok_or(OutputWaitError::NotStarted)?,
                reactor,
            },
        })
    }

    /// Validates a mailbox request before storing it or returning an existing proof.
    /// Canceled requests are removed before the bounded pending list is updated.
    /// Each rejected request gets its own error; rejection cannot issue a proof.
    pub(crate) fn accept(&mut self, request: OutputRequest) {
        let Some(endpoint) = &self.endpoint else {
            request.reply(Err(OutputWaitError::NotStarted));
            return;
        };
        if request.epoch != endpoint.epoch {
            request.reply(Err(OutputWaitError::StaleExecution));
            return;
        }
        if self.enclave != Some(request.target.enclave)
            || self.outcomes.get(request.target.reactor).is_none()
        {
            request.reply(Err(OutputWaitError::InvalidTarget));
            return;
        }
        if let Some(error) = endpoint.lifecycle.error() {
            request.reply(Err(error));
            return;
        }
        self.pending.retain(|request| !request.canceled());
        if request.canceled() {
            return;
        }
        if self.completed.is_some_and(|bound| bound >= request.bound) {
            self.reply(request);
        } else if self.pending.len() < endpoint.capacity {
            self.pending.push(request);
        } else {
            request.reply(Err(OutputWaitError::Capacity));
        }
    }

    /// Removes canceled requests and returns the earliest requested checkpoint.
    /// This candidate is a scheduling input, not evidence of completed execution.
    pub(crate) fn candidate(&mut self) -> Option<Tag> {
        self.pending.retain(|request| !request.canceled());
        self.pending
            .iter()
            .filter_map(|request| match request.bound {
                OutputBound::ThroughTag(tag) => Some(tag),
                OutputBound::ThroughTime(_) => None,
            })
            .min()
    }

    /// Retains an actual direct-owner report without allocating or issuing a proof.
    /// A later failure must not erase a known effect from an earlier invocation.
    /// The latest report replaces the prior slot; no unbounded history is retained.
    pub(crate) fn record(&mut self, owner: ReactorIndex, tag: Tag, outcome: OutputOutcome) {
        self.outcomes[owner] = Some((tag, outcome));
    }

    /// Reads existing direct-owner evidence for scheduler regression tests.
    #[cfg(test)]
    pub(crate) fn retained_outcome(&self, owner: ReactorIndex) -> Option<(Tag, OutputOutcome)> {
        self.outcomes.get(owner).copied().flatten()
    }

    /// Copies an already established scheduler proof into this request's reply.
    /// Call only after validating the request and checking its completed tag.
    fn reply(&self, request: OutputRequest) {
        let completion = OutputCompletion {
            epoch: request.epoch,
            target: request.target,
            bound: request.bound,
        };
        request.reply(Ok(completion));
    }

    /// A caught callback panic leaves no valid continuation of its interrupted tag.
    /// Observer-only failures such as an unsupported backend do not stop execution.
    pub(crate) fn execution_interrupted(&self) -> bool {
        self.execution_interrupted
    }

    /// Fails owned proof state during unwind before the borrowed core disappears.
    pub(crate) fn abort_unwind(&mut self) {
        self.execution_interrupted = true;
        self.close(OutputWaitError::ExecutionFailed);
    }

    /// Publishes a bound for which the scheduler has already established completion.
    ///
    /// The caller must first apply its existing input, coordination, execution,
    /// and mailbox-closure checks. This state is not an independent proof authority.
    /// A tag proof covers that tag, including its microstep, and earlier tags.
    /// It cannot establish closure of all microsteps at the same logical time.
    ///
    /// A known terminal error prevents new success. Earlier replies and retained
    /// outcomes remain intact. Completion uses only the preallocated pending list.
    pub(crate) fn complete_bound(&mut self, bound: OutputBound) {
        if let Some(error) = self
            .endpoint
            .as_ref()
            .and_then(|endpoint| endpoint.lifecycle.error())
        {
            self.close(error);
            return;
        }
        self.completed = Some(self.completed.map_or(bound, |previous| previous.max(bound)));
        let mut index = 0;
        while index < self.pending.len() {
            if self.pending[index].bound <= bound {
                let request = self.pending.swap_remove(index);
                self.reply(request);
            } else {
                index += 1;
            }
        }
    }

    /// Returns the earliest uncanceled time request without changing its cutoff.
    pub(crate) fn time_candidate(&mut self) -> Option<crate::clock::PhysicalInstant> {
        self.pending.retain(|request| !request.canceled());
        self.pending
            .iter()
            .filter_map(|request| match request.bound {
                OutputBound::ThroughTime(time) => Some(time),
                OutputBound::ThroughTag(_) => None,
            })
            .min()
    }

    /// Returns whether a completed public proof covers all microsteps at `time`.
    #[cfg(test)]
    pub(crate) fn time_closed(&self, time: crate::clock::PhysicalInstant) -> bool {
        self.completed
            .is_some_and(|bound| bound >= OutputBound::ThroughTime(time))
    }

    /// Copies the configured route for explicit proof setup only.
    pub(crate) fn proof_endpoint(
        &self,
    ) -> Option<(ExecutionEpoch, usize, crate::Sender<crate::AsyncEvent>)> {
        self.endpoint
            .as_ref()
            .map(|endpoint| (endpoint.epoch, endpoint.capacity, endpoint.event_tx.clone()))
    }

    /// Rejects time requests covered by one failed proof, preserving later waits.
    pub(crate) fn fail_time(
        &mut self,
        time: crate::clock::PhysicalInstant,
        error: OutputWaitError,
    ) {
        let mut index = 0;
        while index < self.pending.len() {
            if matches!(self.pending[index].bound, OutputBound::ThroughTime(t) if t <= time) {
                self.pending.swap_remove(index).reply(Err(error));
            } else {
                index += 1;
            }
        }
    }

    /// Publishes the first terminal error and resolves all retained pending waits.
    /// Mailbox-queued waits read the shared error even before their request is read.
    /// Previously issued replies, completed tags, and actual outcomes are preserved.
    pub(crate) fn close(&mut self, error: OutputWaitError) {
        self.proof.abort(error);
        if let Some(endpoint) = &self.endpoint {
            endpoint.lifecycle.close(error);
        }
        let error = self
            .endpoint
            .as_ref()
            .and_then(|endpoint| endpoint.lifecycle.error())
            .unwrap_or(error);
        for request in self.pending.drain(..) {
            request.reply(Err(error));
        }
    }
}
impl Drop for OutputState {
    fn drop(&mut self) {
        self.close(if std::thread::panicking() {
            OutputWaitError::ExecutionFailed
        } else {
            OutputWaitError::Disconnected
        });
    }
}
