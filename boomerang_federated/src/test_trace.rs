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
enum TraceActor {
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

/// Lossy semantic projection used only to match and display recorded protocol frames.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FramePattern {
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

impl FramePattern {
    fn from_frame(frame: &ProtocolFrame) -> Self {
        match frame {
            ProtocolFrame::FederateToRti(FederateToRti::Hello { .. }) => Self::Hello,
            ProtocolFrame::FederateToRti(FederateToRti::Net { tag, .. }) => Self::Net(*tag),
            ProtocolFrame::FederateToRti(FederateToRti::Ltc { tag, .. }) => Self::Ltc(*tag),
            ProtocolFrame::FederateToRti(FederateToRti::MsgAck { tag, .. }) => Self::MsgAck(*tag),
            ProtocolFrame::FederateToRti(FederateToRti::Msg { endpoint, tag, .. })
            | ProtocolFrame::RtiToFederate(RtiToFederate::Msg { endpoint, tag, .. }) => Self::Msg {
                tag: *tag,
                endpoint: endpoint.clone(),
            },
            ProtocolFrame::FederateToRti(FederateToRti::Stop { .. })
            | ProtocolFrame::RtiToFederate(RtiToFederate::Stop) => Self::Stop,
            ProtocolFrame::RtiToFederate(RtiToFederate::Start { .. }) => Self::Start,
            ProtocolFrame::RtiToFederate(RtiToFederate::Tag { tag }) => Self::Tag(*tag),
            ProtocolFrame::RtiToFederate(RtiToFederate::Error { .. }) => Self::Error,
        }
    }

    fn matches(&self, frame: &ProtocolFrame) -> bool {
        self == &Self::from_frame(frame)
    }
}

impl fmt::Display for FramePattern {
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct TraceEvent {
    client_id: FederateId,
    frame: ProtocolFrame,
}

impl TraceEvent {
    fn new(client_id: FederateId, frame: ProtocolFrame) -> Self {
        Self { client_id, frame }
    }

    fn actors(&self) -> (TraceActor, TraceActor) {
        match &self.frame {
            ProtocolFrame::FederateToRti(_) => {
                (TraceActor::Client(self.client_id.clone()), TraceActor::Rti)
            }
            ProtocolFrame::RtiToFederate(_) => {
                (TraceActor::Rti, TraceActor::Client(self.client_id.clone()))
            }
        }
    }
}

impl fmt::Display for TraceEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (from, to) = self.actors();
        write!(
            f,
            "{from} -> {to} {}",
            FramePattern::from_frame(&self.frame)
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TracePattern {
    from: Option<TraceActor>,
    to: Option<TraceActor>,
    frame: FramePattern,
}

impl TracePattern {
    pub(crate) fn message(frame: FramePattern) -> Self {
        Self {
            from: None,
            to: None,
            frame,
        }
    }

    pub(crate) fn client_to_rti(client_id: FederateId, frame: FramePattern) -> Self {
        Self::between(TraceActor::Client(client_id), TraceActor::Rti, frame)
    }

    pub(crate) fn rti_to_client(client_id: FederateId, frame: FramePattern) -> Self {
        Self::between(TraceActor::Rti, TraceActor::Client(client_id), frame)
    }

    fn between(from: TraceActor, to: TraceActor, frame: FramePattern) -> Self {
        Self {
            from: Some(from),
            to: Some(to),
            frame,
        }
    }

    fn matches(&self, event: &TraceEvent) -> bool {
        let (from, to) = event.actors();
        self.from.as_ref().is_none_or(|expected| expected == &from)
            && self.to.as_ref().is_none_or(|expected| expected == &to)
            && self.frame.matches(&event.frame)
    }
}

impl fmt::Display for TracePattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.from, &self.to) {
            (Some(from), Some(to)) => write!(f, "{from} -> {to} {}", self.frame),
            _ => self.frame.fmt(f),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Trace {
    events: Arc<Mutex<Vec<TraceEvent>>>,
}

impl Trace {
    fn push(&self, event: TraceEvent) {
        self.events
            .lock()
            .expect("trace collector mutex should not be poisoned")
            .push(event);
    }

    fn count(&self, pattern: &TracePattern) -> usize {
        self.events
            .lock()
            .expect("trace collector mutex should not be poisoned")
            .iter()
            .filter(|event| pattern.matches(event))
            .count()
    }

    fn first_position(&self, pattern: &TracePattern) -> Option<usize> {
        self.events
            .lock()
            .expect("trace collector mutex should not be poisoned")
            .iter()
            .position(|event| pattern.matches(event))
    }

    #[track_caller]
    pub(crate) fn assert_before(&self, first: TracePattern, second: TracePattern) {
        let first_position = self.first_position(&first);
        let second_position = self.first_position(&second);

        assert!(
            matches!((first_position, second_position), (Some(first), Some(second)) if first < second),
            "expected `{first}` before `{second}`, found positions {first_position:?} and {second_position:?}\ntrace:\n{}",
            self.normalized()
        );
    }

    #[track_caller]
    pub(crate) fn assert_exact(&self, expected: &[TracePattern]) {
        let events = self
            .events
            .lock()
            .expect("trace collector mutex should not be poisoned");
        let matches = events.len() == expected.len()
            && events
                .iter()
                .zip(expected)
                .all(|(event, pattern)| pattern.matches(event));

        assert!(
            matches,
            "expected exact trace:\n{}actual trace:\n{}",
            Self::normalized_patterns(expected),
            Self::normalized_events(&events)
        );
    }

    #[track_caller]
    pub(crate) fn assert_count(&self, pattern: TracePattern, expected: usize) {
        let actual = self.count(&pattern);
        assert_eq!(
            actual,
            expected,
            "expected {expected} event(s) matching `{pattern}`, found {actual}\ntrace:\n{}",
            self.normalized()
        );
    }

    #[track_caller]
    pub(crate) fn assert_absent(&self, pattern: TracePattern) {
        let actual = self.count(&pattern);
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

    fn normalized_patterns(patterns: &[TracePattern]) -> String {
        let mut output = String::new();
        for (index, pattern) in patterns.iter().enumerate() {
            writeln!(output, "{index}: {pattern}").expect("writing to a String cannot fail");
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
        let frame = ProtocolFrame::FederateToRti(message);
        self.sink
            .send(frame.clone())
            .await
            .expect("recording client transport should send a protocol frame");
        self.trace
            .push(TraceEvent::new(self.federate_id.clone(), frame));
    }

    pub(crate) async fn recv(&mut self) -> RtiToFederate {
        let frame = self
            .stream
            .next()
            .await
            .expect("recording client transport should remain open")
            .expect("recording client transport should receive a protocol frame");
        self.trace
            .push(TraceEvent::new(self.federate_id.clone(), frame.clone()));
        match frame {
            ProtocolFrame::RtiToFederate(message) => message,
            other => panic!("expected RTI-to-federate frame, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::in_memory_transport_pair;

    fn client_to_rti(client_id: &str, message: FederateToRti) -> TraceEvent {
        TraceEvent::new(
            FederateId::from(client_id),
            ProtocolFrame::FederateToRti(message),
        )
    }

    fn rti_to_client(client_id: &str, message: RtiToFederate) -> TraceEvent {
        TraceEvent::new(
            FederateId::from(client_id),
            ProtocolFrame::RtiToFederate(message),
        )
    }

    #[test]
    fn frame_patterns_ignore_non_semantic_protocol_fields() {
        let endpoint = EndpointId::from("output");
        let first = client_to_rti(
            "source",
            FederateToRti::Msg {
                source: FederateId::from("source"),
                target: FederateId::from("target"),
                endpoint: endpoint.clone(),
                tag: WireTag::finite(10, 2),
                payload: vec![1, 2, 3],
            },
        );
        let second = client_to_rti(
            "source",
            FederateToRti::Msg {
                source: FederateId::from("source"),
                target: FederateId::from("another-target"),
                endpoint: endpoint.clone(),
                tag: WireTag::finite(10, 2),
                payload: vec![9],
            },
        );
        let pattern = TracePattern::client_to_rti(
            FederateId::from("source"),
            FramePattern::Msg {
                tag: WireTag::finite(10, 2),
                endpoint,
            },
        );

        assert_ne!(first, second);
        assert!(pattern.matches(&first));
        assert!(pattern.matches(&second));

        let start = rti_to_client(
            "target",
            RtiToFederate::Start {
                start_unix_epoch_ns: 42,
            },
        );
        assert!(
            TracePattern::rti_to_client(FederateId::from("target"), FramePattern::Start)
                .matches(&start)
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
            TracePattern::client_to_rti(federate_id.clone(), FramePattern::Net(WireTag::ZERO)),
            TracePattern::rti_to_client(federate_id, FramePattern::Tag(WireTag::ZERO)),
        ]);
    }

    #[test]
    fn trace_assertions_match_counts_absence_and_causal_order() {
        let source = FederateId::from("source");
        let trace = Trace::default();
        trace.push(client_to_rti(
            "source",
            FederateToRti::Net {
                federate_id: source.clone(),
                tag: WireTag::ZERO,
            },
        ));
        trace.push(rti_to_client(
            "source",
            RtiToFederate::Tag { tag: WireTag::ZERO },
        ));

        let net = TracePattern::client_to_rti(source.clone(), FramePattern::Net(WireTag::ZERO));
        let tag = TracePattern::rti_to_client(source, FramePattern::Tag(WireTag::ZERO));
        assert_eq!(trace.count(&net), 1);
        assert_eq!(trace.first_position(&tag), Some(1));
        trace.assert_count(TracePattern::message(FramePattern::Net(WireTag::ZERO)), 1);
        trace.assert_absent(TracePattern::message(FramePattern::Error));
        trace.assert_exact(&[net.clone(), tag.clone()]);
        trace.assert_before(net, tag);
    }

    #[test]
    fn trace_assertion_failures_include_the_normalized_sequence() {
        let source = FederateId::from("source");
        let trace = Trace::default();
        trace.push(client_to_rti(
            "source",
            FederateToRti::Net {
                federate_id: source,
                tag: WireTag::ZERO,
            },
        ));

        let panic = std::panic::catch_unwind(|| {
            trace.assert_count(TracePattern::message(FramePattern::Net(WireTag::ZERO)), 2);
        })
        .expect_err("the count assertion should fail");
        let message = panic
            .downcast::<String>()
            .expect("assertion failures use an owned String message");

        assert!(message.contains("expected 2 event(s) matching `Net([0ns+0])`, found 1"));
        assert!(message.contains("0: client(source) -> rti Net([0ns+0])"));
    }
}
