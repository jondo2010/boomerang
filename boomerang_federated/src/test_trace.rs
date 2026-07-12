use std::{
    fmt::{self, Write as _},
    sync::{Arc, Mutex},
};

use futures_util::{Sink, SinkExt, Stream, StreamExt};

use crate::{
    protocol::{EndpointId, FederateId, FederateToRti, ProtocolFrame, RtiToFederate, WireTag},
    TransportError,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TraceActor {
    Client(FederateId),
    Rti,
}

impl fmt::Display for TraceActor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Client(federate_id) => write!(f, "client({federate_id})"),
            Self::Rti => f.write_str("rti"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TraceMessage {
    Hello,
    Start,
    Net(WireTag),
    Tag(WireTag),
    Msg { tag: WireTag, endpoint: EndpointId },
    MsgAck(WireTag),
    Ltc(WireTag),
    Stop,
    Error,
}

impl fmt::Display for TraceMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Hello => f.write_str("Hello"),
            Self::Start => f.write_str("Start"),
            Self::Net(tag) => write!(f, "Net({tag})"),
            Self::Tag(tag) => write!(f, "Tag({tag})"),
            Self::Msg { tag, endpoint } => write!(f, "Msg({tag}, {endpoint})"),
            Self::MsgAck(tag) => write!(f, "MsgAck({tag})"),
            Self::Ltc(tag) => write!(f, "Ltc({tag})"),
            Self::Stop => f.write_str("Stop"),
            Self::Error => f.write_str("Error"),
        }
    }
}

impl From<&FederateToRti> for TraceMessage {
    fn from(frame: &FederateToRti) -> Self {
        match frame {
            FederateToRti::Hello { .. } => Self::Hello,
            FederateToRti::Net { tag, .. } => Self::Net(*tag),
            FederateToRti::Ltc { tag, .. } => Self::Ltc(*tag),
            FederateToRti::MsgAck { tag, .. } => Self::MsgAck(*tag),
            FederateToRti::Msg { endpoint, tag, .. } => Self::Msg {
                tag: *tag,
                endpoint: endpoint.clone(),
            },
            FederateToRti::Stop { .. } => Self::Stop,
        }
    }
}

impl From<&RtiToFederate> for TraceMessage {
    fn from(frame: &RtiToFederate) -> Self {
        match frame {
            RtiToFederate::Start { .. } => Self::Start,
            RtiToFederate::Tag { tag } => Self::Tag(*tag),
            RtiToFederate::Msg { endpoint, tag, .. } => Self::Msg {
                tag: *tag,
                endpoint: endpoint.clone(),
            },
            RtiToFederate::Stop => Self::Stop,
            RtiToFederate::Error { .. } => Self::Error,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TraceEvent {
    pub(crate) from: TraceActor,
    pub(crate) to: TraceActor,
    pub(crate) message: TraceMessage,
}

impl TraceEvent {
    pub(crate) fn new(from: TraceActor, to: TraceActor, message: TraceMessage) -> Self {
        Self { from, to, message }
    }

    pub(crate) fn client_to_rti(frame: &FederateToRti) -> Self {
        let federate_id = match frame {
            FederateToRti::Hello { federate_id, .. }
            | FederateToRti::Net { federate_id, .. }
            | FederateToRti::Ltc { federate_id, .. }
            | FederateToRti::MsgAck { federate_id, .. }
            | FederateToRti::Stop { federate_id } => federate_id,
            FederateToRti::Msg { source, .. } => source,
        };

        Self {
            from: TraceActor::Client(federate_id.clone()),
            to: TraceActor::Rti,
            message: frame.into(),
        }
    }

    pub(crate) fn rti_to_client(target: &FederateId, frame: &RtiToFederate) -> Self {
        Self {
            from: TraceActor::Rti,
            to: TraceActor::Client(target.clone()),
            message: frame.into(),
        }
    }
}

impl fmt::Display for TraceEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} -> {} {}", self.from, self.to, self.message)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TracePattern {
    from: Option<TraceActor>,
    to: Option<TraceActor>,
    message: TraceMessage,
}

impl TracePattern {
    pub(crate) fn message(message: TraceMessage) -> Self {
        Self {
            from: None,
            to: None,
            message,
        }
    }

    pub(crate) fn between(from: TraceActor, to: TraceActor, message: TraceMessage) -> Self {
        Self {
            from: Some(from),
            to: Some(to),
            message,
        }
    }

    fn matches(&self, event: &TraceEvent) -> bool {
        self.from.as_ref().is_none_or(|from| from == &event.from)
            && self.to.as_ref().is_none_or(|to| to == &event.to)
            && self.message == event.message
    }
}

impl fmt::Display for TracePattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.from, &self.to) {
            (Some(from), Some(to)) => write!(f, "{from} -> {to} {}", self.message),
            _ => self.message.fmt(f),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Trace {
    events: Arc<Mutex<Vec<TraceEvent>>>,
}

impl Trace {
    pub(crate) fn push(&self, event: TraceEvent) {
        self.events
            .lock()
            .expect("trace collector mutex should not be poisoned")
            .push(event);
    }

    pub(crate) fn count(&self, mut predicate: impl FnMut(&TraceEvent) -> bool) -> usize {
        self.events
            .lock()
            .expect("trace collector mutex should not be poisoned")
            .iter()
            .filter(|event| predicate(event))
            .count()
    }

    pub(crate) fn first_position(
        &self,
        mut predicate: impl FnMut(&TraceEvent) -> bool,
    ) -> Option<usize> {
        self.events
            .lock()
            .expect("trace collector mutex should not be poisoned")
            .iter()
            .position(&mut predicate)
    }

    #[track_caller]
    pub(crate) fn assert_before(&self, first: TracePattern, second: TracePattern) {
        let first_position = self.first_position(|event| first.matches(event));
        let second_position = self.first_position(|event| second.matches(event));

        assert!(
            matches!((first_position, second_position), (Some(first), Some(second)) if first < second),
            "expected `{first}` before `{second}`, found positions {first_position:?} and {second_position:?}\ntrace:\n{}",
            self.normalized()
        );
    }

    #[track_caller]
    pub(crate) fn assert_exact(&self, expected: &[TraceEvent]) {
        let events = self
            .events
            .lock()
            .expect("trace collector mutex should not be poisoned");
        let actual_trace = Self::normalized_events(&events);
        assert_eq!(
            events.as_slice(),
            expected,
            "expected exact trace:\n{}actual trace:\n{}",
            Self::normalized_events(expected),
            actual_trace
        );
    }

    #[track_caller]
    pub(crate) fn assert_count(&self, pattern: TracePattern, expected: usize) {
        let actual = self.count(|event| pattern.matches(event));
        assert_eq!(
            actual,
            expected,
            "expected {expected} event(s) matching `{pattern}`, found {actual}\ntrace:\n{}",
            self.normalized()
        );
    }

    #[track_caller]
    pub(crate) fn assert_absent(&self, pattern: TracePattern) {
        let actual = self.count(|event| pattern.matches(event));
        assert_eq!(
            actual,
            0,
            "expected no events matching `{pattern}`, found {actual}\ntrace:\n{}",
            self.normalized()
        );
    }

    fn normalized(&self) -> String {
        Self::normalized_events(
            &self
                .events
                .lock()
                .expect("trace collector mutex should not be poisoned"),
        )
    }

    fn normalized_events(events: &[TraceEvent]) -> String {
        let mut output = String::new();
        for (index, event) in events.iter().enumerate() {
            writeln!(output, "{index}: {event}").expect("writing to a String cannot fail");
        }
        output
    }
}

/// Test-only client transport decorator that records successful protocol exchanges.
pub(crate) struct RecordingClientTransport<S, St> {
    sink: S,
    stream: St,
    federate_id: FederateId,
    trace: Trace,
}

impl<S, St> RecordingClientTransport<S, St> {
    pub(crate) fn new(transport: (S, St), federate_id: FederateId, trace: Trace) -> Self {
        let (sink, stream) = transport;
        Self {
            sink,
            stream,
            federate_id,
            trace,
        }
    }
}

impl<S, St> RecordingClientTransport<S, St>
where
    S: Sink<ProtocolFrame> + Unpin,
    S::Error: fmt::Debug,
    St: Stream<Item = Result<ProtocolFrame, TransportError>> + Unpin,
{
    pub(crate) async fn send(&mut self, message: FederateToRti) {
        let event = TraceEvent::client_to_rti(&message);
        self.sink
            .send(ProtocolFrame::FederateToRti(message))
            .await
            .expect("recording client transport should send a protocol frame");
        self.trace.push(event);
    }

    pub(crate) async fn recv(&mut self) -> RtiToFederate {
        let frame = self
            .stream
            .next()
            .await
            .expect("recording client transport should remain open")
            .expect("recording client transport should receive a protocol frame");
        let message = match frame {
            ProtocolFrame::RtiToFederate(message) => message,
            other => panic!("expected RTI-to-federate frame, got {other:?}"),
        };
        self.trace
            .push(TraceEvent::rti_to_client(&self.federate_id, &message));
        message
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::in_memory_transport_pair;

    fn client(id: &str) -> TraceActor {
        TraceActor::Client(FederateId::from(id))
    }

    #[test]
    fn trace_normalizes_frames_to_semantic_fields() {
        let first = FederateToRti::Msg {
            source: FederateId::from("source"),
            target: FederateId::from("target"),
            endpoint: EndpointId::from("output"),
            tag: WireTag::finite(10, 2),
            payload: vec![1, 2, 3],
        };
        let second = FederateToRti::Msg {
            source: FederateId::from("source"),
            target: FederateId::from("another-target"),
            endpoint: EndpointId::from("output"),
            tag: WireTag::finite(10, 2),
            payload: vec![9],
        };

        assert_eq!(
            TraceEvent::client_to_rti(&first),
            TraceEvent::client_to_rti(&second)
        );

        let start = RtiToFederate::Start {
            start_unix_epoch_ns: 42,
        };
        assert_eq!(
            TraceEvent::rti_to_client(&FederateId::from("target"), &start).message,
            TraceMessage::Start
        );
    }

    #[tokio::test]
    async fn recording_transport_captures_successful_protocol_frames() {
        let federate_id = FederateId::from("source");
        let (client, mut rti) = in_memory_transport_pair();
        let trace = Trace::default();
        let mut client = RecordingClientTransport::new(client, federate_id.clone(), trace.clone());

        client
            .send(FederateToRti::Net {
                federate_id: federate_id.clone(),
                tag: WireTag::ZERO,
            })
            .await;
        assert_eq!(
            rti.1.next().await.unwrap().unwrap(),
            ProtocolFrame::FederateToRti(FederateToRti::Net {
                federate_id: federate_id.clone(),
                tag: WireTag::ZERO,
            })
        );

        rti.0
            .send(ProtocolFrame::RtiToFederate(RtiToFederate::Tag {
                tag: WireTag::ZERO,
            }))
            .await
            .unwrap();
        assert_eq!(
            client.recv().await,
            RtiToFederate::Tag { tag: WireTag::ZERO }
        );

        trace.assert_exact(&[
            TraceEvent::new(
                TraceActor::Client(federate_id.clone()),
                TraceActor::Rti,
                TraceMessage::Net(WireTag::ZERO),
            ),
            TraceEvent::new(
                TraceActor::Rti,
                TraceActor::Client(federate_id),
                TraceMessage::Tag(WireTag::ZERO),
            ),
        ]);
    }

    #[test]
    fn trace_assertions_match_counts_absence_and_causal_order() {
        let trace = Trace::default();
        trace.push(TraceEvent {
            from: client("source"),
            to: TraceActor::Rti,
            message: TraceMessage::Net(WireTag::ZERO),
        });
        trace.push(TraceEvent {
            from: TraceActor::Rti,
            to: client("source"),
            message: TraceMessage::Tag(WireTag::ZERO),
        });

        assert_eq!(
            trace.count(|event| matches!(event.message, TraceMessage::Net(_))),
            1
        );
        assert_eq!(
            trace.first_position(|event| matches!(event.message, TraceMessage::Tag(_))),
            Some(1)
        );
        trace.assert_count(TracePattern::message(TraceMessage::Net(WireTag::ZERO)), 1);
        trace.assert_absent(TracePattern::message(TraceMessage::Error));
        trace.assert_exact(&[
            TraceEvent::new(
                client("source"),
                TraceActor::Rti,
                TraceMessage::Net(WireTag::ZERO),
            ),
            TraceEvent::new(
                TraceActor::Rti,
                client("source"),
                TraceMessage::Tag(WireTag::ZERO),
            ),
        ]);
        trace.assert_before(
            TracePattern::between(
                client("source"),
                TraceActor::Rti,
                TraceMessage::Net(WireTag::ZERO),
            ),
            TracePattern::between(
                TraceActor::Rti,
                client("source"),
                TraceMessage::Tag(WireTag::ZERO),
            ),
        );
    }

    #[test]
    fn trace_assertion_failures_include_the_normalized_sequence() {
        let trace = Trace::default();
        trace.push(TraceEvent {
            from: client("source"),
            to: TraceActor::Rti,
            message: TraceMessage::Net(WireTag::ZERO),
        });

        let panic = std::panic::catch_unwind(|| {
            trace.assert_count(TracePattern::message(TraceMessage::Net(WireTag::ZERO)), 2);
        })
        .expect_err("the count assertion should fail");
        let message = panic
            .downcast::<String>()
            .expect("assertion failures use an owned String message");

        assert!(message.contains("expected 2 event(s) matching `Net([0ns+0])`, found 1"));
        assert!(message.contains("0: client(source) -> rti Net([0ns+0])"));
    }
}
