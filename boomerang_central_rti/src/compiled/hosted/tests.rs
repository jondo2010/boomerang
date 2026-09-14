//! Real-socket admission, framing, and lifecycle regression coverage.
use super::*;
use canonical::Reply;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

/// Opens a connected local socket pair without background accept workers.
fn sockets() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    (client, listener.accept().unwrap().0)
}

/// Converts a standard loopback socket on its owning Tokio executor.
fn framed(
    stream: TcpStream,
    timeout: Duration,
) -> (tokio::runtime::Runtime, io::Reader, io::Writer) {
    let rt = runtime().unwrap();
    stream.set_nonblocking(true).unwrap();
    let (reader, writer) =
        rt.block_on(async { io::split(tokio::net::TcpStream::from_std(stream).unwrap(), timeout) });
    (rt, reader, writer)
}
/// Retains the receiver so a synchronous producer can exercise real Tokio channel admission.
fn test_sink() -> (HostedSink, mpsc::Receiver<Envelope<RtiRequest>>) {
    let (shared, requests, _) = shared(Duration::from_secs(1));
    (HostedSink(shared), requests)
}

#[test]
fn connection_drop_joins_worker_even_with_live_sink() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let connection = connect(
        listener.local_addr().unwrap(),
        "source",
        Duration::from_secs(1),
    )
    .unwrap();
    let sink = connection.sink();
    let start = Instant::now();
    drop(connection);
    assert!(start.elapsed() < Duration::from_secs(1));
    assert!(sink.send(RtiRequest::Stop).is_err());
}

/// Runs a two-member standalone image with borrowed tables inside its owning thread.
fn server(timeout: Duration) -> (SocketAddr, JoinHandle<Result<(), CentralRtiError>>) {
    use boomerang_runtime::image::{
        IdentityTable, RecoveryPolicy, RtiImage, RtiImageView, RtiMemberImage,
    };
    use tinymap::{SliceRange, TinyMapView};
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let worker = thread::spawn(move || {
        let members = [
            RtiMemberImage::new(
                RecoveryPolicy::FailStop,
                SliceRange::new(0, 0),
                SliceRange::new(0, 0),
                SliceRange::new(0, 0),
            ),
            RtiMemberImage::new(
                RecoveryPolicy::FailStop,
                SliceRange::new(0, 0),
                SliceRange::new(0, 0),
                SliceRange::new(0, 0),
            ),
        ];
        let image = RtiImage::new(
            TinyMapView::new(&members),
            &[],
            &[],
            TinyMapView::new(&[]),
            IdentityTable::new(&[]),
            IdentityTable::new(&[]),
            IdentityTable::new(&[]),
            IdentityTable::new(&[]),
        );
        let view = RtiImageView::new(&image, IdentityTable::new(&["source", "target"])).unwrap();
        let rti = CompiledRti::from_image(&view, CoordinationIdentity::new([1; 32])).unwrap();
        serve(listener, rti, test_contract(), timeout)
    });
    (address, worker)
}

/// Submits the fingerprint used by the test image.
fn hello(connection: &HostedConnection) {
    connection
        .sink()
        .send(RtiRequest::Hello {
            identity: CoordinationIdentity::new([1; 32]),
        })
        .unwrap();
}

#[test]
fn socket_members_admit_and_stop_using_core_authority() {
    let timeout = Duration::from_secs(2);
    let (address, server) = server(timeout);
    let mut source = connect(address, "source", timeout).unwrap();
    let mut target = connect(address, "target", timeout).unwrap();
    hello(&source);
    hello(&target);
    assert!(matches!(
        source.receive(timeout).unwrap(),
        Some(RtiReply::Started)
    ));
    assert!(matches!(
        target.receive(timeout).unwrap(),
        Some(RtiReply::Started)
    ));
    for peer in [&source, &target] {
        peer.sink()
            .send(RtiRequest::Publish {
                revision: 1,
                next_event: None,
            })
            .unwrap();
        peer.sink()
            .send(RtiRequest::ConfirmIdle { revision: 1 })
            .unwrap();
    }
    for peer in [&mut source, &mut target] {
        assert!(matches!(
            peer.receive(timeout).unwrap(),
            Some(RtiReply::Idle { revision: 1 })
        ));
        peer.sink().send(RtiRequest::Stop).unwrap();
        assert!(matches!(
            peer.receive(timeout).unwrap(),
            Some(RtiReply::Stopped)
        ));
        peer.shutdown().unwrap();
    }
    assert!(server.join().unwrap().is_ok());
}

#[test]
fn fingerprint_mismatch_releases_other_admitted_peer() {
    let timeout = Duration::from_secs(1);
    let (address, server) = server(timeout);
    let contract = test_contract();
    let (source, mut session) = admitted_peer(address, timeout, &contract);
    let (rt, mut source, _writer) = framed(source, timeout);
    let target = connect(address, "target", timeout).unwrap();
    target
        .sink()
        .send(RtiRequest::Hello {
            identity: CoordinationIdentity::new([2; 32]),
        })
        .unwrap();
    let bytes = rt.block_on(source.receive()).unwrap();
    assert!(
        matches!(session.decode(&bytes).unwrap(), canonical::Message::Reply(Reply::Failed { message }) if message.contains("fingerprint mismatch"))
    );
    assert!(server
        .join()
        .unwrap()
        .unwrap_err()
        .to_string()
        .contains("fingerprint mismatch"));
}

#[test]
fn disconnect_releases_a_peer_waiting_for_admission() {
    let timeout = Duration::from_secs(1);
    let (address, server) = server(timeout);
    let mut source = connect(address, "source", timeout).unwrap();
    hello(&source);
    // Waiting briefly confirms the first peer's bind reached the server before its peer vanishes.
    assert!(source.receive(Duration::from_millis(20)).unwrap().is_none());
    let target = connect(address, "target", timeout).unwrap();
    drop(target);
    let start = Instant::now();
    assert!(matches!(
        source.receive(timeout).unwrap(),
        Some(RtiReply::Failed { .. })
    ));
    assert!(server.join().unwrap().is_err());
    assert!(start.elapsed() < timeout);
}

#[test]
fn missing_member_admission_has_a_bounded_deadline() {
    let timeout = Duration::from_millis(80);
    let (address, server) = server(timeout);
    let mut source = connect(address, "source", Duration::from_secs(1)).unwrap();
    hello(&source);
    let start = Instant::now();
    assert!(
        matches!(source.receive(Duration::from_secs(1)).unwrap(), Some(RtiReply::Failed { message }) if message.contains("admission timed out"))
    );
    assert!(server.join().unwrap().is_err());
    assert!(start.elapsed() < Duration::from_secs(1));
}

#[test]
fn outbound_payload_size_is_bounded_before_submission() {
    let (sink, _requests) = test_sink();
    assert!(sink
        .send(RtiRequest::Payload {
            route: RtiRouteIndex::new(0),
            tag: WireTag::ZERO,
            payload: vec![0; MAX_FRAME_BYTES]
        })
        .is_err());
}

#[test]
fn json_codec_bounds_payload_before_transport_submission() {
    let bytes = encode_json(&42_u32).unwrap();
    assert_eq!(decode_json::<u32>(&bytes).unwrap(), 42);
    assert!(encode_json(&"x".repeat(MAX_FRAME_BYTES)).is_err());
    assert!(
        decode_json::<String>(format!("\"{}\"", "x".repeat(MAX_FRAME_BYTES)).as_bytes()).is_err()
    );
}

#[test]
fn healthy_idle_connections_outlive_the_operation_timeout() {
    let timeout = Duration::from_millis(60);
    let (address, server) = server(timeout);
    let mut source = connect(address, "source", Duration::from_secs(1)).unwrap();
    let mut target = connect(address, "target", Duration::from_secs(1)).unwrap();
    hello(&source);
    hello(&target);
    assert!(matches!(
        source.receive(Duration::from_secs(1)).unwrap(),
        Some(RtiReply::Started)
    ));
    assert!(matches!(
        target.receive(Duration::from_secs(1)).unwrap(),
        Some(RtiReply::Started)
    ));
    assert!(source
        .receive(Duration::from_millis(120))
        .unwrap()
        .is_none());
    for peer in [&source, &target] {
        peer.sink()
            .send(RtiRequest::Publish {
                revision: 1,
                next_event: None,
            })
            .unwrap();
        peer.sink()
            .send(RtiRequest::ConfirmIdle { revision: 1 })
            .unwrap();
    }
    for peer in [&mut source, &mut target] {
        assert!(matches!(
            peer.receive(Duration::from_secs(1)).unwrap(),
            Some(RtiReply::Idle { .. })
        ));
        peer.sink().send(RtiRequest::Stop).unwrap();
        assert!(matches!(
            peer.receive(Duration::from_secs(1)).unwrap(),
            Some(RtiReply::Stopped)
        ));
    }
    server.join().unwrap().unwrap();
}

#[test]
fn request_sink_fails_without_blocking_when_its_bounded_queue_is_full() {
    use std::error::Error;
    let (sink, mut requests) = test_sink();
    for _ in 0..QUEUE_CAPACITY {
        sink.send(RtiRequest::Stop).unwrap();
    }
    let start = Instant::now();
    let original = sink.send(RtiRequest::Stop).unwrap_err();
    assert!(original
        .source()
        .unwrap()
        .source()
        .unwrap()
        .is::<channel::ChannelError>());
    assert!(start.elapsed() < Duration::from_millis(100));
    assert!(sink.0.state.lock().unwrap().closing);
    requests.try_recv().unwrap();
    assert_eq!(
        sink.send(RtiRequest::Stop).unwrap_err().to_string(),
        original.to_string()
    );
}

#[test]
fn unknown_and_duplicate_stable_members_fail_closed() {
    for member in ["missing", "source"] {
        let timeout = Duration::from_secs(1);
        let (address, server) = server(timeout);
        let mut source = connect(address, "source", timeout).unwrap();
        hello(&source);
        assert!(source.receive(Duration::from_millis(20)).unwrap().is_none());
        let mut rejected = TcpStream::connect(address).unwrap();
        let contract = test_contract();
        let mut hello = contract.handshake(FederateIndex::new(0)).unwrap();
        hello.member = member;
        let mut bytes = vec![0; MAX_FRAME_BYTES];
        let count = canonical::encode_handshake(&hello, &mut bytes).unwrap();
        rejected.write_all(&bytes[..count]).unwrap();
        assert!(matches!(
            source.receive(timeout).unwrap(),
            Some(RtiReply::Failed { .. })
        ));
        assert!(server.join().unwrap().is_err());
    }
}

#[test]
fn dense_route_requests_before_fingerprint_admission_fail_closed() {
    let timeout = Duration::from_secs(1);
    let (address, server) = server(timeout);
    let mut source = connect(address, "source", timeout).unwrap();
    source
        .sink()
        .send(RtiRequest::Payload {
            route: RtiRouteIndex::new(0),
            tag: WireTag::ZERO,
            payload: vec![42],
        })
        .unwrap();
    assert!(source
        .receive(timeout)
        .unwrap_err()
        .to_string()
        .contains("handshake has not completed"));
    assert!(server.join().unwrap().is_err());
}

#[test]
fn dropping_owner_flushes_an_accepted_abort_before_disconnect() {
    let timeout = Duration::from_secs(1);
    let (address, server) = server(timeout);
    let mut source = connect(address, "source", timeout).unwrap();
    let mut target = connect(address, "target", timeout).unwrap();
    hello(&source);
    hello(&target);
    assert!(matches!(
        source.receive(timeout).unwrap(),
        Some(RtiReply::Started)
    ));
    assert!(matches!(
        target.receive(timeout).unwrap(),
        Some(RtiReply::Started)
    ));
    source
        .sink()
        .send(RtiRequest::Abort {
            message: "original local scheduler failure".into(),
        })
        .unwrap();
    drop(source);
    assert!(
        matches!(target.receive(timeout).unwrap(), Some(RtiReply::Failed { message }) if message.contains("original local scheduler failure"))
    );
    assert!(server
        .join()
        .unwrap()
        .unwrap_err()
        .to_string()
        .contains("original local scheduler failure"));
}

#[test]
fn closing_drain_joins_within_one_deadline_when_the_peer_does_not_read() {
    let timeout = Duration::from_millis(80);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut connection = connect(listener.local_addr().unwrap(), "source", timeout).unwrap();
    let _unread_peer = listener.accept().unwrap().0;
    let sink = connection.sink();
    hello(&connection);
    for _ in 0..boomerang_federated::channel::PAYLOAD_CAPACITY {
        sink.send(RtiRequest::Payload {
            route: RtiRouteIndex::new(0),
            tag: WireTag::ZERO,
            payload: vec![42; MAX_PAYLOAD_BYTES],
        })
        .unwrap();
    }
    let start = Instant::now();
    assert!(connection
        .shutdown()
        .unwrap_err()
        .to_string()
        .contains("timed out"));
    assert!(start.elapsed() < Duration::from_millis(500));
    assert!(sink.send(RtiRequest::Stop).is_err());
}

/// Uses the same borrowed typed domain as the standalone test coordinator.
pub(super) fn test_contract() -> WireContract<'static> {
    test_contract_routes(&[])
}

/// Binds a test route table without manufacturing or casting its typed keys.
fn test_contract_routes(
    routes: &'static [boomerang_runtime::image::RtiRouteImage<'static>],
) -> WireContract<'static> {
    use boomerang_federated::wire::{Contract, CoordinationFingerprint};
    use tinymap::TinyMapView;
    Contract::new(
        CoordinationFingerprint::new([1; 32]),
        [3; 32],
        TinyMapView::new(&["source", "target"]),
        TinyMapView::new(routes),
        |route: &boomerang_runtime::image::RtiRouteImage<'_>| (route.source(), route.target()),
    )
}

#[test]
fn actual_hosted_server_echoes_exact_canonical_preflight() {
    let timeout = Duration::from_secs(1);
    let (address, server) = server(timeout);
    let contract = test_contract();
    let (mut peer, mut session) = admitted_peer(address, timeout, &contract);
    peer.write_all(&encode(&mut session, &canonical::Message::Reply(Reply::Started)).unwrap())
        .unwrap();
    assert!(server
        .join()
        .unwrap()
        .unwrap_err()
        .to_string()
        .contains("channel direction"));
}

/// Waits for the exact handshake echo before returning an admitted source peer.
fn admitted_peer<'a>(
    address: SocketAddr,
    timeout: Duration,
    contract: &'a WireContract<'static>,
) -> (TcpStream, WireSession<'a, 'static>) {
    use boomerang_federated::wire;
    let mut peer = TcpStream::connect(address).unwrap();
    peer.set_read_timeout(Some(timeout)).unwrap();
    let hello = contract.handshake(FederateIndex::new(0)).unwrap();
    let mut bytes = vec![0; wire::MAX_FRAME_BYTES];
    let count = wire::encode_handshake(&hello, &mut bytes).unwrap();
    bytes.truncate(count);
    peer.write_all(&bytes).unwrap();
    let mut echo = vec![0; count];
    peer.read_exact(&mut echo).unwrap();
    assert_eq!(echo, bytes);
    let mut session = WireSession::new(contract, FederateIndex::new(0)).unwrap();
    session.accept_handshake(&echo).unwrap();
    (peer, session)
}

/// Binds a test roster name once at the hosted adapter entrypoint.
fn connect(
    address: SocketAddr,
    member: &str,
    timeout: Duration,
) -> Result<HostedConnection, CentralRtiError> {
    let member = match member {
        "source" => FederateIndex::new(0),
        "target" => FederateIndex::new(1),
        _ => panic!("invalid test roster member"),
    };
    super::connect(address, member, test_contract(), timeout)
}

#[test]
fn missing_handshake_echo_has_an_absolute_admission_deadline() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut connection = connect(
        listener.local_addr().unwrap(),
        "source",
        Duration::from_millis(30),
    )
    .unwrap();
    let _silent = listener.accept().unwrap().0;
    hello(&connection);
    let error = connection.receive(Duration::from_millis(300)).unwrap_err();
    assert!(error.to_string().contains("admission timed out"));
    assert_eq!(
        connection.shutdown().unwrap_err().to_string(),
        error.to_string()
    );
}

#[test]
fn accepting_abort_closes_producer_admission() {
    let (sink, _requests) = test_sink();
    sink.send(RtiRequest::Abort {
        message: "first cause".into(),
    })
    .unwrap();
    assert!(sink.send(RtiRequest::Stop).is_err());
}

#[test]
fn retained_requests_do_not_keep_unbounded_spare_capacity() {
    let (sink, mut requests) = test_sink();
    let mut payload = Vec::with_capacity(MAX_FRAME_BYTES * 4);
    payload.push(42);
    sink.send(RtiRequest::Payload {
        route: RtiRouteIndex::new(0),
        tag: WireTag::ZERO,
        payload,
    })
    .unwrap();
    let Some(RtiRequest::Payload { payload, .. }) =
        requests.try_recv().ok().map(Envelope::into_value)
    else {
        panic!("missing payload")
    };
    assert!(payload.capacity() <= MAX_PAYLOAD_BYTES);
    let mut message = String::with_capacity(MAX_FRAME_BYTES * 4);
    message.push('x');
    sink.send(RtiRequest::Abort { message }).unwrap();
    let Some(RtiRequest::Abort { message }) = requests.try_recv().ok().map(Envelope::into_value)
    else {
        panic!("missing diagnostic")
    };
    assert!(message.capacity() <= canonical::MAX_DIAGNOSTIC_BYTES);
}

#[test]
fn maximum_accepted_abort_reaches_healthy_peer_unchanged() {
    for message in [
        "x".repeat(canonical::MAX_DIAGNOSTIC_BYTES),
        "é".repeat(canonical::MAX_DIAGNOSTIC_BYTES / 2),
    ] {
        let timeout = Duration::from_secs(1);
        let (address, server) = server(timeout);
        let mut source = connect(address, "source", timeout).unwrap();
        let mut target = connect(address, "target", timeout).unwrap();
        hello(&source);
        hello(&target);
        assert!(matches!(
            source.receive(timeout).unwrap(),
            Some(RtiReply::Started)
        ));
        assert!(matches!(
            target.receive(timeout).unwrap(),
            Some(RtiReply::Started)
        ));
        source
            .sink()
            .send(RtiRequest::Abort {
                message: message.clone(),
            })
            .unwrap();
        assert!(
            matches!(target.receive(timeout).unwrap(), Some(RtiReply::Failed { message: received }) if received == message)
        );
        assert!(server
            .join()
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains(&message));
    }
}

#[test]
fn abort_drains_a_stalled_socket_despite_inbound_coordination() {
    let timeout = Duration::from_secs(2);
    let (mut sender, mut peer) = sockets();
    sender.set_nonblocking(true).unwrap();
    peer.set_read_timeout(Some(timeout)).unwrap();
    let mut filled = 0;
    loop {
        match sender.write(&[0; 8192]) {
            Ok(count) => {
                filled += count;
                assert!(filled < 16 * 1024 * 1024);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
            result => panic!("unexpected socket fill: {result:?}"),
        }
    }
    let contract = test_contract();
    let mut bytes = vec![0; MAX_FRAME_BYTES];
    let count = canonical::encode_handshake(
        &contract.handshake(FederateIndex::new(0)).unwrap(),
        &mut bytes,
    )
    .unwrap();
    peer.write_all(&bytes[..count]).unwrap();
    let mut session = WireSession::new(&contract, FederateIndex::new(0)).unwrap();
    session.accept_handshake(&bytes[..count]).unwrap();
    peer.write_all(
        &encode(
            &mut session,
            &canonical::Message::Reply(canonical::Reply::Started),
        )
        .unwrap(),
    )
    .unwrap();
    let (shared, requests, replies) = shared(timeout);
    let sink = HostedSink(shared.clone());
    sink.send(RtiRequest::Hello {
        identity: CoordinationIdentity::new([1; 32]),
    })
    .unwrap();
    sink.send(RtiRequest::Abort {
        message: "drained cause".into(),
    })
    .unwrap();
    shared.close();
    let worker_shared = shared.clone();
    let worker = thread::spawn(move || {
        runtime().unwrap().block_on(async {
            let stream = tokio::net::TcpStream::from_std(sender).unwrap();
            client_loop(
                stream,
                FederateIndex::new(0),
                &test_contract(),
                &worker_shared,
                requests,
                replies,
            )
            .await
        })
    });
    std::io::copy(
        &mut Read::by_ref(&mut peer).take(filled as u64),
        &mut std::io::sink(),
    )
    .unwrap();
    peer.read_exact(&mut bytes[..count]).unwrap();
    peer.read_exact(&mut bytes[..canonical::FRAME_PREFIX_BYTES])
        .unwrap();
    let length = canonical::frame_length(&bytes[..canonical::FRAME_PREFIX_BYTES])
        .unwrap()
        .unwrap();
    peer.read_exact(&mut bytes[canonical::FRAME_PREFIX_BYTES..length])
        .unwrap();
    assert!(matches!(
        session.decode(&bytes[..length]).unwrap(),
        canonical::Message::Request(canonical::Request::Abort {
            message: "drained cause"
        })
    ));
    peer.shutdown(std::net::Shutdown::Write).unwrap();
    worker.join().unwrap().unwrap();
}

#[test]
fn remote_failure_closes_admission_and_cancels_stalled_output() {
    let timeout = Duration::from_millis(150);
    let (mut sender, mut peer) = runtime().unwrap().block_on(async {
        let listener = tokio::net::TcpSocket::new_v4().unwrap();
        listener.set_recv_buffer_size(1024).unwrap();
        listener.bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let listener = listener.listen(1).unwrap();
        let client = tokio::net::TcpSocket::new_v4().unwrap();
        client.set_send_buffer_size(1024).unwrap();
        let (sender, peer) = tokio::join!(
            client.connect(listener.local_addr().unwrap()),
            listener.accept()
        );
        (
            sender.unwrap().into_std().unwrap(),
            peer.unwrap().0.into_std().unwrap(),
        )
    });
    peer.set_nonblocking(false).unwrap();
    sender.set_nonblocking(true).unwrap();
    while sender.write(&[0; 8192]).is_ok() {}
    use boomerang_runtime::image::*;
    static ROUTES: [RtiRouteImage; 1] = [RtiRouteImage::new(
        BoundaryId::new("pipe"),
        FlowIndex::new(0),
        None,
        None,
        BoundaryFailurePolicy::PropagateStop,
        TransportPolicy::ReliableOrderedFramed,
        CodecPolicy::CanonicalBounded,
        TimingPolicy::BestEffort,
        SecurityPolicy::None,
        TransportCapabilityIndex::new(0),
        CodecCapabilityIndex::new(0),
        FederateIndex::new(0),
        FederateIndex::new(1),
        0,
    )];
    let contract = test_contract_routes(&ROUTES);
    let mut bytes = vec![0; MAX_FRAME_BYTES];
    let count = canonical::encode_handshake(
        &contract.handshake(FederateIndex::new(0)).unwrap(),
        &mut bytes,
    )
    .unwrap();
    peer.write_all(&bytes[..count]).unwrap();
    let mut session = WireSession::new(&contract, FederateIndex::new(0)).unwrap();
    session.accept_handshake(&bytes[..count]).unwrap();
    let failed = encode(
        &mut session,
        &canonical::Message::Reply(Reply::Failed {
            message: "original remote failure",
        }),
    )
    .unwrap();
    let (shared, requests, replies) = shared(timeout);
    let sink = HostedSink(shared.clone());
    sink.send(RtiRequest::Hello {
        identity: CoordinationIdentity::new([1; 32]),
    })
    .unwrap();
    for _ in 0..boomerang_federated::channel::PAYLOAD_CAPACITY {
        sink.send(RtiRequest::Payload {
            route: RtiRouteIndex::new(0),
            tag: WireTag::ZERO,
            payload: vec![42; MAX_PAYLOAD_BYTES],
        })
        .unwrap();
    }
    runtime().unwrap().block_on(async {
        let stream = tokio::net::TcpStream::from_std(sender).unwrap();
        let client = client_loop(
            stream,
            FederateIndex::new(0),
            &contract,
            &shared,
            requests,
            replies,
        );
        let remote = async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            peer.write_all(&failed).unwrap();
        };
        let (result, ()) = tokio::join!(client, remote);
        result.unwrap();
    });
    assert!(sink.send(RtiRequest::Stop).is_err());
    assert!(
        matches!(shared.state.lock().unwrap().replies.try_recv().unwrap().into_value(), RtiReply::Failed { message } if message == "original remote failure")
    );
}

#[test]
fn reply_burst_waits_for_delayed_synchronous_consumer() {
    let timeout = Duration::from_secs(2);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut connection = connect(listener.local_addr().unwrap(), "source", timeout).unwrap();
    hello(&connection);
    let remote = thread::spawn(move || {
        let (mut peer, _) = listener.accept().unwrap();
        peer.set_read_timeout(Some(timeout)).unwrap();
        let mut prefix = [0; canonical::FRAME_PREFIX_BYTES];
        peer.read_exact(&mut prefix).unwrap();
        let mut bytes = vec![0; canonical::frame_length(&prefix).unwrap().unwrap()];
        bytes[..prefix.len()].copy_from_slice(&prefix);
        peer.read_exact(&mut bytes[prefix.len()..]).unwrap();
        let contract = test_contract();
        let mut session = WireSession::new(&contract, FederateIndex::new(0)).unwrap();
        session.accept_handshake(&bytes).unwrap();
        for revision in 0..QUEUE_CAPACITY * 3 {
            bytes.extend(
                encode(
                    &mut session,
                    &canonical::Message::Reply(Reply::Idle {
                        revision: revision as u64,
                    }),
                )
                .unwrap(),
            );
        }
        bytes.extend(
            encode(
                &mut session,
                &canonical::Message::Reply(Reply::Failed {
                    message: "burst complete",
                }),
            )
            .unwrap(),
        );
        peer.write_all(&bytes).unwrap();
        std::io::copy(&mut peer, &mut std::io::sink()).unwrap();
    });
    thread::sleep(Duration::from_millis(50));
    for revision in 0..QUEUE_CAPACITY * 3 {
        assert!(
            matches!(connection.receive(timeout).unwrap(), Some(RtiReply::Idle { revision: actual }) if actual == revision as u64)
        );
    }
    assert!(
        matches!(connection.receive(timeout).unwrap(), Some(RtiReply::Failed { message }) if message == "burst complete")
    );
    connection.shutdown().unwrap();
    remote.join().unwrap();
}
