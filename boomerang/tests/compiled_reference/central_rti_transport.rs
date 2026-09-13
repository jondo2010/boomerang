//! Fixture-specific RTI worker and fault injection over the reference in-memory transport.
use super::*;
use boomerang::central_rti::compiled::{
    in_memory::{InMemoryReceiver, InMemorySender},
    RtiRequest, RtiRequestSink,
};
use std::{sync::mpsc, time::Duration};

/// One reference in-memory client connection.
type Connection = (Arc<dyn RtiRequestSink>, InMemoryReceiver);
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
            Arc::new(InMemorySender::new(source, tx.clone())),
            InMemoryReceiver::new(source_rx),
        ),
        (
            Arc::new(InMemorySender::new(sink, tx)),
            InMemoryReceiver::new(sink_rx),
        ),
        server,
    )
}
