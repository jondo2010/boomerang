//! Real-socket admission, framing, and lifecycle regression coverage.
use super::*;
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

/// Opens a connected local socket pair without background accept workers.
fn sockets() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    (client, listener.accept().unwrap().0)
}

#[test]
fn malformed_and_oversize_frames_are_rejected_on_real_sockets() {
    for (bytes, diagnostic) in [
        (vec![0, 0, 0, 1, 255], "unknown hosted frame type"),
        (
            (MAX_FRAME_BYTES as u32 + 1).to_be_bytes().to_vec(),
            "invalid hosted frame length",
        ),
    ] {
        let (mut sender, receiver) = sockets();
        let mut receiver = FramedSocket::new(receiver, Duration::from_millis(100)).unwrap();
        sender.write_all(&bytes).unwrap();
        let start = Instant::now();
        let error = loop {
            match receiver.receive() {
                Err(error) => break error,
                Ok(_) => {
                    assert!(start.elapsed() < Duration::from_secs(1));
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        };
        assert!(error.to_string().contains(diagnostic), "{error}");
    }
}

#[test]
fn partial_frame_has_a_fixed_deadline() {
    let (mut sender, receiver) = sockets();
    let mut receiver = FramedSocket::new(receiver, Duration::from_millis(20)).unwrap();
    sender.write_all(&[0, 0]).unwrap();
    let start = Instant::now();
    let error = loop {
        match receiver.receive() {
            Err(error) => break error,
            Ok(_) => {
                assert!(start.elapsed() < Duration::from_secs(1));
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    };
    assert!(error.to_string().contains("frame read timed out"));
    assert!(start.elapsed() >= Duration::from_millis(20));
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
        serve(listener, rti, timeout)
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
    let mut source = connect(address, "source", timeout).unwrap();
    let target = connect(address, "target", timeout).unwrap();
    hello(&source);
    target
        .sink()
        .send(RtiRequest::Hello {
            identity: CoordinationIdentity::new([2; 32]),
        })
        .unwrap();
    assert!(
        matches!(source.receive(timeout).unwrap(), Some(RtiReply::Failed { message }) if message.contains("identity mismatch"))
    );
    assert!(server
        .join()
        .unwrap()
        .unwrap_err()
        .to_string()
        .contains("identity mismatch"));
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
fn outbound_frame_size_and_queue_are_bounded() {
    let (sender, _) = sockets();
    let mut socket = FramedSocket::new(sender, Duration::from_secs(1)).unwrap();
    for _ in 0..QUEUE_CAPACITY {
        socket
            .queue(encode(Frame::Request(RtiRequest::Stop)).unwrap())
            .unwrap();
    }
    assert!(socket
        .queue(encode(Frame::Request(RtiRequest::Stop)).unwrap())
        .is_err());
    assert!(encode(Frame::Request(RtiRequest::Payload {
        route: RtiRouteIndex::new(0),
        tag: WireTag::ZERO,
        payload: vec![0; MAX_FRAME_BYTES]
    }))
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
    let (requests, _receiver) = mpsc::sync_channel(QUEUE_CAPACITY);
    let sink = HostedSink {
        requests,
        closed: Arc::new(AtomicBool::new(false)),
    };
    for _ in 0..QUEUE_CAPACITY {
        sink.send(RtiRequest::Stop).unwrap();
    }
    let start = Instant::now();
    assert!(sink
        .send(RtiRequest::Stop)
        .unwrap_err()
        .to_string()
        .contains("queue"));
    assert!(start.elapsed() < Duration::from_millis(100));
    assert!(sink.closed.load(Ordering::Acquire));
}

#[test]
fn unknown_and_duplicate_stable_members_fail_closed() {
    for member in ["missing", "source"] {
        let timeout = Duration::from_secs(1);
        let (address, server) = server(timeout);
        let mut source = connect(address, "source", timeout).unwrap();
        hello(&source);
        assert!(source.receive(Duration::from_millis(10)).unwrap().is_none());
        let _rejected = connect(address, member, timeout).unwrap();
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
    assert!(
        matches!(source.receive(timeout).unwrap(), Some(RtiReply::Failed { message }) if message.contains("before fingerprint admission"))
    );
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
    for _ in 0..QUEUE_CAPACITY {
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
