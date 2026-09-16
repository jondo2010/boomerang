use super::*;
use core::error::Error as _;
tinymap::key_type!(Member);
tinymap::key_type!(Route);
static MEMBERS: [&str; 2] = ["alpha", "beta"];
static ROUTES: [(Member, Member); 2] = [(Member::new(0), Member::new(1)); 2];
fn contract() -> Contract<'static, Member, Route, (Member, Member)> {
    Contract::new(
        CoordinationFingerprint::new([1; 32]),
        [2; 32],
        TinyMapView::new(&MEMBERS),
        TinyMapView::new(&ROUTES),
        |v| (v.0, v.1),
    )
}
fn hello(member: &str) -> Handshake<'_> {
    Handshake {
        protocol: 1,
        codec: CODEC_VERSION,
        coordination: CoordinationFingerprint::new([1; 32]),
        epoch: 0,
        incarnation: 0,
        mapping: [2; 32],
        member,
    }
}
fn preflight(session: &mut Session<'_, '_, Member, Route, (Member, Member)>, member: &str) {
    let mut bytes = [0; 512];
    let n = encode_handshake(&hello(member), &mut bytes).unwrap();
    session.accept_handshake(&bytes[..n]).unwrap();
}
fn frame(body: &[u8]) -> Vec<u8> {
    let mut out = u32::try_from(body.len()).unwrap().to_be_bytes().to_vec();
    out.extend_from_slice(body);
    out
}
/// Flags, ordinary-record discriminator, and reserved epoch/incarnation precede the message.
fn traffic(body: &[u8]) -> Vec<u8> {
    frame(&[&[0, 1, 0, 0][..], body].concat())
}
#[test]
fn baseline_messages_have_canonical_vectors_and_borrow_payloads() {
    let c = contract();
    let tag = WireTag::finite(0x010203, 0x040506);
    let tag_bytes = [1, 0x86, 0x88, 8, 0x86, 0x8a, 0x10];
    let cases = [
        (
            Message::Request(Request::Publish {
                revision: 300,
                next_event: None,
            }),
            vec![0, 0, 0xac, 2, 0],
        ),
        (
            Message::Request(Request::Publish {
                revision: 300,
                next_event: Some(tag),
            }),
            [&[0, 0, 0xac, 2, 1][..], &tag_bytes].concat(),
        ),
        (
            Message::Request(Request::Complete {
                tag: WireTag::finite(-1, 2),
            }),
            vec![0, 1, 1, 1, 2],
        ),
        (
            Message::Request(Request::Payload {
                route: Route::new(1),
                tag,
                payload: &[42],
            }),
            [&[0, 2, 1][..], &tag_bytes, &[1, 42]].concat(),
        ),
        (
            Message::Request(Request::ConfirmIdle { revision: 300 }),
            vec![0, 3, 0xac, 2],
        ),
        (Message::Request(Request::Stop), vec![0, 4]),
        (
            Message::Request(Request::Abort { message: "x" }),
            vec![0, 5, 1, b'x'],
        ),
        (Message::Reply(Reply::Started), vec![1, 0]),
        (
            Message::Reply(Reply::Grant { revision: 300, tag }),
            [&[1, 1, 0xac, 2][..], &tag_bytes].concat(),
        ),
        (
            Message::Reply(Reply::Payload {
                route: Route::new(1),
                tag,
                payload: &[42],
            }),
            [&[1, 2, 1][..], &tag_bytes, &[1, 42]].concat(),
        ),
        (
            Message::Reply(Reply::Idle { revision: 300 }),
            vec![1, 3, 0xac, 2],
        ),
        (Message::Reply(Reply::Stopped), vec![1, 4]),
        (
            Message::Reply(Reply::Dnet { tag }),
            [&[1, 6][..], &tag_bytes].concat(),
        ),
        (
            Message::Reply(Reply::Failed { message: "x" }),
            vec![1, 5, 1, b'x'],
        ),
    ];
    for kind in [0, 2] {
        let mut s = Session::new(&c, Member::new(0)).unwrap();
        preflight(&mut s, "alpha");
        let encoded = traffic(&[0, 1, kind]);
        let message = s.decode(&encoded).unwrap();
        let mut output = [0; 64];
        let n = s.encode(&message, &mut output).unwrap();
        assert_eq!(&output[..n], encoded);
    }
    for (message, body) in cases {
        let is_delivery = matches!(message, Message::Reply(Reply::Payload { .. }));
        let (peer, id) = if is_delivery {
            (Member::new(1), "beta")
        } else {
            (Member::new(0), "alpha")
        };
        let mut sender = Session::new(&c, peer).unwrap();
        let mut receiver = Session::new(&c, peer).unwrap();
        preflight(&mut sender, id);
        preflight(&mut receiver, id);
        let expected = traffic(&body);
        let mut output = [0; 512];
        let n = sender.encode(&message, &mut output).unwrap();
        assert_eq!(&output[..n], expected);
        let decoded = receiver.decode(&expected).unwrap();
        assert_eq!(decoded, message);
        if let Message::Request(Request::Payload { payload, .. })
        | Message::Reply(Reply::Payload { payload, .. }) = decoded
        {
            assert_eq!(payload.as_ptr(), expected[expected.len() - 1..].as_ptr());
        }
    }
}
#[test]
fn handshake_vector_and_every_identity_mismatch_fail_closed() {
    let c = contract();
    let mut buffer = [0; 512];
    let n = encode_handshake(&hello("alpha"), &mut buffer).unwrap();
    let expected = frame(
        &[
            &[0, 0, 1, 1][..],
            &[1; 32],
            &[0, 0],
            &[2; 32],
            &[5, b'a', b'l', b'p', b'h', b'a'],
        ]
        .concat(),
    );
    assert_eq!(&buffer[..n], expected);
    for (offset, failure) in [
        (6, WireError::Admission(AdmissionError::Protocol)),
        (7, WireError::Admission(AdmissionError::Codec)),
        (8, WireError::Admission(AdmissionError::Coordination)),
        (40, WireError::Frame(FrameError::Epoch)),
        (41, WireError::Frame(FrameError::Epoch)),
        (42, WireError::Admission(AdmissionError::Mapping)),
        (75, WireError::Admission(AdmissionError::Peer)),
    ] {
        let mut bytes = expected.clone();
        bytes[offset] ^= 1;
        let mut s = Session::new(&c, Member::new(0)).unwrap();
        assert_eq!(s.accept_handshake(&bytes), Err(failure));
        assert_eq!(s.accept_handshake(&expected), Err(WireError::SessionFailed));
    }
    let mut wrong_peer = Session::new(&c, Member::new(1)).unwrap();
    assert_eq!(
        wrong_peer.accept_handshake(&expected),
        Err(WireError::Admission(AdmissionError::Peer))
    );
    let mut early = Session::new(&c, Member::new(0)).unwrap();
    assert_eq!(early.decode(&traffic(&[1, 0])), Err(WireError::NotAdmitted));
    assert_eq!(
        early.accept_handshake(&expected),
        Err(WireError::SessionFailed)
    );
}
#[test]
fn malformed_frames_cannot_admit_routes_or_survive_failure() {
    let c = contract();
    let valid = traffic(&[0, 2, 0, 1, 0, 0, 0]);
    let mut admitted = Session::new(&c, Member::new(0)).unwrap();
    preflight(&mut admitted, "alpha");
    assert!(admitted.decode(&valid).is_ok());
    let mut bad_frames = (0..valid.len())
        .map(|n| valid[..n].to_vec())
        .collect::<Vec<_>>();
    for n in 4..valid.len() {
        let mut b = valid[..n].to_vec();
        b[..4].copy_from_slice(&u32::try_from(n - 4).unwrap().to_be_bytes());
        bad_frames.push(b);
    }
    bad_frames.push(traffic(&[0, 6])); // Local Hello is not a wire traffic opcode.
    for kind in [6, 7, 255] {
        bad_frames.push(traffic(&[1, kind]));
    }
    for offset in [4, 5, 6, 7, 8, 9, 10, 11, 14] {
        let mut b = valid.clone();
        b[offset] = 255;
        bad_frames.push(b);
    }
    let mut b = valid.clone();
    b.push(0);
    bad_frames.push(b);
    for bytes in bad_frames {
        let mut s = Session::new(&c, Member::new(0)).unwrap();
        preflight(&mut s, "alpha");
        assert!(s.decode(&bytes).is_err(), "accepted {bytes:?}");
        assert_eq!(s.decode(&traffic(&[1, 0])), Err(WireError::SessionFailed));
    }
    let mut s = Session::new(&c, Member::new(1)).unwrap();
    preflight(&mut s, "beta");
    assert_eq!(
        s.decode(&valid),
        Err(WireError::Admission(AdmissionError::Route))
    );
}
#[test]
fn bounds_are_checked_without_size_driven_storage() {
    let c = contract();
    let mut s = Session::new(&c, Member::new(0)).unwrap();
    preflight(&mut s, "alpha");
    assert_eq!(
        s.encode(
            &Message::Request(Request::Hello {
                identity: CoordinationFingerprint::new([1; 32])
            }),
            &mut [0; 128]
        ),
        Err(WireError::Frame(FrameError::Unsupported))
    );
    let mut s = Session::new(&c, Member::new(0)).unwrap();
    preflight(&mut s, "alpha");
    let error = s.decode(&[255; 4]).unwrap_err();
    assert_eq!(error, WireError::Frame(FrameError::Oversize));
    assert!(error.source().unwrap().is::<FrameError>());
    let mut s = Session::new(&c, Member::new(0)).unwrap();
    preflight(&mut s, "alpha");
    let oversized = traffic(
        &[
            &[0, 2, 0, 0, 0x80, 0x80, 4][..],
            &[0; MAX_PAYLOAD_BYTES + 1],
        ]
        .concat(),
    );
    assert_eq!(
        s.decode(&oversized),
        Err(WireError::Frame(FrameError::Oversize))
    );
    let bytes = [0; MAX_PAYLOAD_BYTES + 1];
    let mut s = Session::new(&c, Member::new(0)).unwrap();
    preflight(&mut s, "alpha");
    assert_eq!(
        s.encode(
            &Message::Request(Request::Payload {
                route: Route::new(0),
                tag: WireTag::ZERO,
                payload: &bytes
            }),
            &mut [0; 8]
        ),
        Err(WireError::Frame(FrameError::Oversize))
    );
    let mut s = Session::new(&c, Member::new(0)).unwrap();
    preflight(&mut s, "alpha");
    assert_eq!(
        s.encode(&Message::Reply(Reply::Started), &mut [0; 8]),
        Err(WireError::Frame(FrameError::Truncated))
    );
    let mut s = Session::new(&c, Member::new(0)).unwrap();
    preflight(&mut s, "alpha");
    let mut storage = [0; MAX_FRAME_BYTES];
    let n = s
        .encode(
            &Message::Request(Request::Payload {
                route: Route::new(0),
                tag: WireTag::ZERO,
                payload: &bytes[..MAX_PAYLOAD_BYTES],
            }),
            &mut storage,
        )
        .unwrap();
    assert!(n <= MAX_FRAME_BYTES);
    assert!(s.decode(&storage[..n]).is_ok());
    let largest = Message::Request(Request::Payload {
        route: u32::MAX,
        tag: WireTag::finite(i128::MIN, u64::MAX),
        payload: &bytes[..MAX_PAYLOAD_BYTES],
    });
    let record = Record::<Handshake<'_>, _>::Message {
        epoch: 0,
        incarnation: 0,
        message: largest,
    };
    assert_eq!(
        encode_frame(&record, &mut storage).unwrap(),
        MAX_FRAME_BYTES
    );
    assert!(decode_frame(&storage).is_ok());
}

#[test]
fn invalid_rosters_and_terminal_failure_cannot_reopen_admission() {
    let c = contract();
    assert!(Session::new(&c, Member::new(2)).is_err());
    for members in [["alpha", "alpha"], ["beta", "alpha"], ["", "beta"]] {
        let invalid = Contract::<Member, Route, _>::new(
            CoordinationFingerprint::new([1; 32]),
            [2; 32],
            TinyMapView::new(&members),
            TinyMapView::new(&ROUTES),
            |v| (v.0, v.1),
        );
        assert!(matches!(
            Session::new(&invalid, Member::new(0)),
            Err(WireError::Admission(AdmissionError::Mapping))
        ));
    }
    for direction in [0, 1] {
        let mut s = Session::new(&c, Member::new(0)).unwrap();
        preflight(&mut s, "alpha");
        assert!(s.decode(&traffic(&[direction, 5, 0])).is_ok());
        assert_eq!(s.decode(&traffic(&[1, 0])), Err(WireError::SessionFailed));
    }
    for bytes in [
        traffic(&[0, 5, 1, 255]),     // Invalid UTF-8.
        traffic(&[0, 0, 0, 2]),       // Invalid Option discriminant.
        traffic(&[0, 0, 0x80, 0, 0]), // Overlong revision.
        traffic(&[0x80, 0, 0, 0, 0]), // Overlong enum discriminant.
        traffic(&[0, 5, 0x80, 0]),    // Overlong string length.
        traffic(&[0, 1, 0, 0]),       // Trailing bytes after Never.
    ] {
        let mut s = Session::new(&c, Member::new(0)).unwrap();
        preflight(&mut s, "alpha");
        assert!(s.decode(&bytes).is_err(), "accepted {bytes:?}");
    }
}

#[test]
fn stream_prefix_is_bounded_before_a_body_is_available() {
    assert_eq!(frame_length(&[0, 0]).unwrap(), None);
    assert_eq!(frame_length(&[0, 0, 0, 8]).unwrap(), Some(12));
    assert!(frame_length(&u32::MAX.to_be_bytes()).is_err());
    assert!(frame_length(&[0, 0, 0, 0]).is_err());
}
