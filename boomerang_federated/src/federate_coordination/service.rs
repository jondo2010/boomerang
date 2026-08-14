use std::{collections::BTreeMap, sync::mpsc, thread::JoinHandle, time::Duration};

use boomerang_runtime::{
    AsyncEvent, CommonContext, CoordinationError, CoordinationOutcome, CoordinationWake,
    EnclaveKey, FrontierPublication, LogicalTimeCoordinator, SendContext, Tag,
};

use crate::client::coordination::{ProtocolPoll, RtiLogicalTimeCoordinator};

use super::{
    state::{CoordinationAction, FederateCoordinationState},
    FederateCoordinationLayout,
};

type Reply = mpsc::Sender<Result<(), String>>;

#[derive(Debug)]
enum Request {
    Publish {
        participant: EnclaveKey,
        sequence: u64,
        publication: FrontierPublication,
        reply: Reply,
    },
    Acquire {
        participant: EnclaveKey,
        request_id: u64,
        publication_sequence: u64,
        tag: Tag,
        reply: Reply,
    },
    Complete {
        participant: EnclaveKey,
        request_id: u64,
        tag: Tag,
        reply: Reply,
    },
    ForceStop {
        reply: Reply,
    },
}

pub(crate) struct FederateCoordinationService {
    requests: mpsc::Sender<Request>,
    worker: JoinHandle<Result<RtiLogicalTimeCoordinator, String>>,
}

impl FederateCoordinationService {
    pub(crate) fn spawn(
        coordinator: RtiLogicalTimeCoordinator,
        layout: FederateCoordinationLayout,
        wakes: BTreeMap<EnclaveKey, SendContext>,
    ) -> Result<Self, std::io::Error> {
        let (requests_tx, requests_rx) = mpsc::channel();
        let worker = std::thread::Builder::new()
            .name("federate-coordination".into())
            .spawn(move || {
                let result = run(coordinator, layout, wakes, requests_rx);
                if let Err(error) = &result {
                    tracing::error!(%error, "Federate coordination service failed");
                }
                result
            })?;
        Ok(Self {
            requests: requests_tx,
            worker,
        })
    }

    pub(crate) fn participant(&self, participant: EnclaveKey) -> FederateParticipantProxy {
        FederateParticipantProxy {
            participant,
            requests: self.requests.clone(),
            publication_sequence: 0,
            request_id: 0,
        }
    }

    pub(crate) fn force_stop(&self) -> Result<(), String> {
        let (tx, rx) = mpsc::channel();
        self.requests
            .send(Request::ForceStop { reply: tx })
            .map_err(|_| "coordination service stopped".to_owned())?;
        rx.recv()
            .map_err(|_| "coordination service stopped".to_owned())?
    }

    pub(crate) fn join(self) -> Result<RtiLogicalTimeCoordinator, String> {
        self.worker.join().map_err(panic_payload_message)?
    }
}

pub(crate) struct FederateParticipantProxy {
    participant: EnclaveKey,
    requests: mpsc::Sender<Request>,
    publication_sequence: u64,
    request_id: u64,
}

impl std::fmt::Debug for FederateParticipantProxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FederateParticipantProxy")
            .field("participant", &self.participant)
            .finish()
    }
}

impl FederateParticipantProxy {
    fn call(&self, request: impl FnOnce(Reply) -> Request) -> Result<(), CoordinationError> {
        let (tx, rx) = mpsc::channel();
        self.requests
            .send(request(tx))
            .map_err(|_| CoordinationError::new("Federate coordination service stopped"))?;
        rx.recv()
            .map_err(|_| CoordinationError::new("Federate coordination service stopped"))?
            .map_err(CoordinationError::new)
    }
}

impl LogicalTimeCoordinator for FederateParticipantProxy {
    fn publish_frontier(
        &mut self,
        publication: FrontierPublication,
    ) -> Result<(), CoordinationError> {
        self.publication_sequence = self
            .publication_sequence
            .checked_add(1)
            .ok_or_else(|| CoordinationError::new("publication sequence overflow"))?;
        let participant = self.participant;
        let sequence = self.publication_sequence;
        self.call(|reply| Request::Publish {
            participant,
            sequence,
            publication,
            reply,
        })
    }

    fn acquire(
        &mut self,
        tag: Tag,
        event_rx: &boomerang_runtime::Receiver<AsyncEvent>,
    ) -> Result<CoordinationOutcome, CoordinationError> {
        self.request_id = self
            .request_id
            .checked_add(1)
            .ok_or_else(|| CoordinationError::new("request id overflow"))?;
        let (tx, rx) = mpsc::channel();
        self.requests
            .send(Request::Acquire {
                participant: self.participant,
                request_id: self.request_id,
                publication_sequence: self.publication_sequence,
                tag,
                reply: tx,
            })
            .map_err(|_| CoordinationError::new("Federate coordination service stopped"))?;
        loop {
            match rx.try_recv() {
                Ok(Ok(())) => return Ok(CoordinationOutcome::Granted),
                Ok(Err(message)) => return Err(CoordinationError::new(message)),
                Err(mpsc::TryRecvError::Disconnected) => {
                    return Err(CoordinationError::new(
                        "Federate coordination service stopped",
                    ))
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
            if let Ok(event) = event_rx.recv_timeout(Duration::from_millis(1)) {
                return Ok(CoordinationOutcome::Interrupted(event));
            }
        }
    }

    fn complete(&mut self, tag: Tag) -> Result<(), CoordinationError> {
        self.request_id = self
            .request_id
            .checked_add(1)
            .ok_or_else(|| CoordinationError::new("request id overflow"))?;
        let participant = self.participant;
        let request_id = self.request_id;
        self.call(|reply| Request::Complete {
            participant,
            request_id,
            tag,
            reply,
        })
    }
}

fn run(
    mut coordinator: RtiLogicalTimeCoordinator,
    layout: FederateCoordinationLayout,
    wakes: BTreeMap<EnclaveKey, SendContext>,
    requests: mpsc::Receiver<Request>,
) -> Result<RtiLogicalTimeCoordinator, String> {
    let mut state = FederateCoordinationState::new(layout);
    let mut pending = BTreeMap::<(EnclaveKey, u64), Reply>::new();
    let mut pending_wakes = BTreeMap::<EnclaveKey, CoordinationWake>::new();
    loop {
        flush_pending_wakes(&wakes, &mut pending_wakes);
        while let Ok(request) = requests.try_recv() {
            tracing::debug!(?request, "applying Federate coordination request");
            match request {
                Request::Publish {
                    participant,
                    sequence,
                    publication,
                    reply,
                } => match state.publish(participant, sequence, publication) {
                    Ok(actions) => {
                        if let Err(error) = execute_actions(
                            actions,
                            &mut state,
                            &mut coordinator,
                            &wakes,
                            &mut pending,
                            &mut pending_wakes,
                        ) {
                            let first = terminate(
                                &mut state,
                                &mut coordinator,
                                &mut pending,
                                &mut pending_wakes,
                                error,
                            );
                            let _ = reply.send(Err(first.clone()));
                            return Err(first);
                        }
                        let _ = reply.send(Ok(()));
                    }
                    Err(error) => {
                        let _ = reply.send(Err(error));
                    }
                },
                Request::Acquire {
                    participant,
                    request_id,
                    publication_sequence,
                    tag,
                    reply,
                } => {
                    pending.insert((participant, request_id), reply);
                    let actions =
                        match state.acquire(participant, request_id, publication_sequence, tag) {
                            Ok(actions) => actions,
                            Err(error) => {
                                return Err(terminate(
                                    &mut state,
                                    &mut coordinator,
                                    &mut pending,
                                    &mut pending_wakes,
                                    error,
                                ));
                            }
                        };
                    if let Err(error) = execute_actions(
                        actions,
                        &mut state,
                        &mut coordinator,
                        &wakes,
                        &mut pending,
                        &mut pending_wakes,
                    ) {
                        return Err(terminate(
                            &mut state,
                            &mut coordinator,
                            &mut pending,
                            &mut pending_wakes,
                            error,
                        ));
                    }
                }
                Request::Complete {
                    participant,
                    request_id,
                    tag,
                    reply,
                } => {
                    pending.insert((participant, request_id), reply);
                    match state.complete(participant, request_id, tag) {
                        Ok(actions) => {
                            if let Err(error) = execute_actions(
                                actions,
                                &mut state,
                                &mut coordinator,
                                &wakes,
                                &mut pending,
                                &mut pending_wakes,
                            ) {
                                return Err(terminate(
                                    &mut state,
                                    &mut coordinator,
                                    &mut pending,
                                    &mut pending_wakes,
                                    error,
                                ));
                            }
                        }
                        Err(error) => {
                            if let Some(reply) = pending.remove(&(participant, request_id)) {
                                let _ = reply.send(Err(error));
                            }
                        }
                    }
                }
                Request::ForceStop { reply } => {
                    let actions = state.fail("Federate coordination stopped".into());
                    let result = execute_actions(
                        actions,
                        &mut state,
                        &mut coordinator,
                        &wakes,
                        &mut pending,
                        &mut pending_wakes,
                    );
                    let _ = reply.send(result.clone());
                    return result.map(|_| coordinator);
                }
            }
            if state.is_stopped() {
                return Ok(coordinator);
            }
        }
        match coordinator.poll() {
            Ok(ProtocolPoll::Pending | ProtocolPoll::Progress) => {}
            Ok(ProtocolPoll::Granted(tag)) => {
                if let Err(error) = execute_actions(
                    state.grant(tag),
                    &mut state,
                    &mut coordinator,
                    &wakes,
                    &mut pending,
                    &mut pending_wakes,
                ) {
                    return Err(terminate(
                        &mut state,
                        &mut coordinator,
                        &mut pending,
                        &mut pending_wakes,
                        error,
                    ));
                }
            }
            Err(error) => {
                return Err(terminate(
                    &mut state,
                    &mut coordinator,
                    &mut pending,
                    &mut pending_wakes,
                    error.to_string(),
                ));
            }
        }
    }
}

fn execute_actions(
    actions: Vec<CoordinationAction>,
    _state: &mut FederateCoordinationState,
    coordinator: &mut RtiLogicalTimeCoordinator,
    wakes: &BTreeMap<EnclaveKey, SendContext>,
    pending: &mut BTreeMap<(EnclaveKey, u64), Reply>,
    pending_wakes: &mut BTreeMap<EnclaveKey, CoordinationWake>,
) -> Result<(), String> {
    for action in actions {
        tracing::debug!(?action, "executing Federate coordination action");
        match action {
            CoordinationAction::RequestNet { tag } => coordinator
                .submit_net(tag)
                .map_err(|error| error.to_string())?,
            CoordinationAction::WakeParticipant {
                participant,
                tag,
                observation_epoch,
            } => {
                if !wakes.contains_key(&participant) {
                    return Err(format!("missing wake context for {participant}"));
                }
                pending_wakes.insert(
                    participant,
                    CoordinationWake {
                        tag,
                        observation_epoch,
                    },
                );
            }
            CoordinationAction::ReleaseAcquire {
                participant,
                request_id,
                ..
            } => {
                _state.release_request(participant, request_id);
                if let Some(reply) = pending.remove(&(participant, request_id)) {
                    let _ = reply.send(Ok(()));
                }
            }
            CoordinationAction::ReleaseCompletion {
                participant,
                request_id,
            } => {
                _state.release_request(participant, request_id);
                if let Some(reply) = pending.remove(&(participant, request_id)) {
                    let _ = reply.send(Ok(()));
                }
            }
            CoordinationAction::FailRequest {
                participant,
                request_id,
                reason,
            } => {
                _state.release_request(participant, request_id);
                if let Some(reply) = pending.remove(&(participant, request_id)) {
                    let _ = reply.send(Err(reason));
                }
            }
            CoordinationAction::ReportLtc { tag } => coordinator
                .report_logical_tag_complete(tag)
                .map_err(|error| error.to_string())?,
            CoordinationAction::SendStop => {
                coordinator.stop().map_err(|error| error.to_string())?
            }
        }
    }
    flush_pending_wakes(wakes, pending_wakes);
    Ok(())
}

fn terminate(
    state: &mut FederateCoordinationState,
    coordinator: &mut RtiLogicalTimeCoordinator,
    pending: &mut BTreeMap<(EnclaveKey, u64), Reply>,
    pending_wakes: &mut BTreeMap<EnclaveKey, CoordinationWake>,
    reason: String,
) -> String {
    let actions = state.fail(reason);
    let first = state
        .terminal_error()
        .expect("fail records a terminal error")
        .to_owned();
    for action in actions {
        match action {
            CoordinationAction::FailRequest {
                participant,
                request_id,
                reason,
            } => {
                if let Some(reply) = pending.remove(&(participant, request_id)) {
                    let _ = reply.send(Err(reason));
                }
            }
            CoordinationAction::SendStop => {
                let _ = coordinator.stop();
            }
            _ => unreachable!("terminal state emitted a non-terminal action"),
        }
    }
    pending_wakes.clear();
    fail_all(pending, first.clone());
    first
}

fn flush_pending_wakes(
    wakes: &BTreeMap<EnclaveKey, SendContext>,
    pending_wakes: &mut BTreeMap<EnclaveKey, CoordinationWake>,
) {
    pending_wakes.retain(|participant, wake| {
        let Some(context) = wakes.get(participant) else {
            return false;
        };
        match context.try_schedule_async(AsyncEvent::CoordinationWake(*wake)) {
            Some(true) => false,
            Some(false) => {
                tracing::debug!(%participant, "participant wake raced scheduler shutdown");
                false
            }
            None => true,
        }
    });
}

fn fail_all(pending: &mut BTreeMap<(EnclaveKey, u64), Reply>, message: String) -> String {
    for (_, reply) in std::mem::take(pending) {
        let _ = reply.send(Err(message.clone()));
    }
    message
}

fn panic_payload_message(payload: Box<dyn std::any::Any + Send + 'static>) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|value| (*value).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "coordination service panicked".into())
}
