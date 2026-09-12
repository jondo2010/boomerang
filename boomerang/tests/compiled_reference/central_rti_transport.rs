//! Test-only channel wiring. This is not the intended production transport or hot path.
use super::*;
use boomerang::central_rti::compiled::{
    CentralRtiError, RtiReply, RtiReplySource, RtiRequest, RtiRequestSink,
};
use std::{sync::mpsc, time::Duration};

/// Test sender carrying a server-local authenticated member binding.
struct TestSender {
    /// Server-local binding resolved once from the stable member identity.
    member: FederateIndex,
    /// Test-only request channel shared with the RTI worker.
    tx: mpsc::Sender<(FederateIndex, RtiRequest)>,
}
impl RtiRequestSink for TestSender {
    fn send(&self, request: RtiRequest) -> Result<(), CentralRtiError> {
        self.tx
            .send((self.member, request))
            .map_err(|error| CentralRtiError::new(error.to_string()))
    }
}
/// Test receiver for one member's ordered replies.
pub(super) struct TestReceiver(mpsc::Receiver<RtiReply>);
impl RtiReplySource for TestReceiver {
    fn receive(&mut self, timeout: Duration) -> Result<Option<RtiReply>, CentralRtiError> {
        match self.0.recv_timeout(timeout) {
            Ok(reply) => Ok(Some(reply)),
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(error) => Err(CentralRtiError::new(error.to_string())),
        }
    }
}
/// One test-only client connection; no channel type appears in production APIs.
type Connection = (Arc<dyn RtiRequestSink>, TestReceiver);
/// Drives the production RTI transition engine, forwarding every reply in order.
pub(super) fn start(
    mut rti: CompiledRti<'static>,
    inject_failure: bool,
) -> (Connection, Connection, std::thread::JoinHandle<()>) {
    let (tx, rx) = mpsc::channel();
    let (source_tx, source_rx) = mpsc::channel();
    let (sink_tx, sink_rx) = mpsc::channel();
    let source = rti.resolve_member("a-source").unwrap();
    let sink = rti.resolve_member("b-sink").unwrap();
    let replies = [(source, source_tx), (sink, sink_tx)]
        .into_iter()
        .collect::<std::collections::BTreeMap<_, _>>();
    let server = std::thread::spawn(move || {
        while !rti.is_finished() {
            let (member, request) = rx.recv_timeout(Duration::from_secs(3)).unwrap();
            let delivery = if inject_failure && matches!(request, RtiRequest::Publish { .. }) {
                rti.abort("injected RTI failure")
            } else {
                rti.handle(member, request)
            };
            for delivery in delivery {
                let _ = replies[&delivery.member].send(delivery.reply);
            }
        }
        // Keep the request half open until clients consume the ordered terminal reply
        // and release their sinks; otherwise a racing publication hides that diagnostic.
        while rx.recv_timeout(Duration::from_secs(3)).is_ok() {}
    });
    (
        (
            Arc::new(TestSender {
                member: source,
                tx: tx.clone(),
            }),
            TestReceiver(source_rx),
        ),
        (
            Arc::new(TestSender { member: sink, tx }),
            TestReceiver(sink_rx),
        ),
        server,
    )
}
