use super::*;
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
        codec: 1,
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
    let mut out = u32::try_from(18 + body.len() - 1)
        .unwrap()
        .to_be_bytes()
        .to_vec();
    out.push(body[0]);
    out.extend_from_slice(&[0; 17]);
    out.extend_from_slice(&body[1..]);
    out
}
#[test]
fn baseline_messages_have_fixed_canonical_vectors_and_borrow_payloads() {
    let c = contract();
    let tag = WireTag::finite(0x010203, 0x040506);
    let tag_bytes: [u8; 25] = [
        1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 3, 0, 0, 0, 0, 0, 4, 5, 6,
    ];
    let cases = [
        (
            Message::Publish {
                revision: 0x0102030405060708,
                next_event: None,
            },
            vec![1, 1, 2, 3, 4, 5, 6, 7, 8, 0],
        ),
        (
            Message::Publish {
                revision: 0x0102030405060708,
                next_event: Some(tag),
            },
            [&[1, 1, 2, 3, 4, 5, 6, 7, 8, 1][..], &tag_bytes].concat(),
        ),
        (
            Message::Complete {
                tag: WireTag::finite(-1, 2),
            },
            [&[2, 1][..], &[255; 16], &[0, 0, 0, 0, 0, 0, 0, 2]].concat(),
        ),
        (
            Message::PayloadToRti {
                route: Route::new(1),
                tag,
                payload: &[42],
            },
            [&[3, 0, 0, 0, 1][..], &tag_bytes, &[0, 0, 0, 1, 42]].concat(),
        ),
        (
            Message::ConfirmIdle {
                revision: 0x0102030405060708,
            },
            vec![4, 1, 2, 3, 4, 5, 6, 7, 8],
        ),
        (Message::Stop, vec![5]),
        (Message::Abort { message: "x" }, vec![6, 0, 1, b'x']),
        (Message::Started, vec![7]),
        (
            Message::Grant {
                revision: 0x0102030405060708,
                tag,
            },
            [&[8, 1, 2, 3, 4, 5, 6, 7, 8][..], &tag_bytes].concat(),
        ),
        (
            Message::PayloadToFederate {
                route: Route::new(1),
                tag,
                payload: &[42],
            },
            [&[9, 0, 0, 0, 1][..], &tag_bytes, &[0, 0, 0, 1, 42]].concat(),
        ),
        (
            Message::Idle {
                revision: 0x0102030405060708,
            },
            vec![10, 1, 2, 3, 4, 5, 6, 7, 8],
        ),
        (Message::Stopped, vec![11]),
        (Message::Failed { message: "x" }, vec![12, 0, 1, b'x']),
    ];
    for kind in [0, 2] {
        let mut s = Session::new(&c, Member::new(0)).unwrap();
        preflight(&mut s, "alpha");
        let encoded = frame(&[&[2, kind][..], &[0; 24]].concat());
        let message = s.decode(&encoded).unwrap();
        let mut output = [0; 64];
        let n = s.encode(&message, &mut output).unwrap();
        assert_eq!(&output[..n], encoded);
    }
    for (message, body) in cases {
        let is_delivery = matches!(message, Message::PayloadToFederate { .. });
        let (peer, id) = if is_delivery {
            (Member::new(1), "beta")
        } else {
            (Member::new(0), "alpha")
        };
        let mut sender = Session::new(&c, peer).unwrap();
        let mut receiver = Session::new(&c, peer).unwrap();
        preflight(&mut sender, id);
        preflight(&mut receiver, id);
        let expected = frame(&body);
        let mut output = [0; 512];
        let n = sender.encode(&message, &mut output).unwrap();
        assert_eq!(&output[..n], expected);
        let decoded = receiver.decode(&expected).unwrap();
        assert_eq!(decoded, message);
        if let Message::PayloadToRti { payload, .. } | Message::PayloadToFederate { payload, .. } =
            decoded
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
            &[0, 0, 1, 0, 1][..],
            &[1; 32],
            &[2; 32],
            &[0, 5, b'a', b'l', b'p', b'h', b'a'],
        ]
        .concat(),
    );
    assert_eq!(&buffer[..n], expected);
    for (offset, failure) in [
        (22, WireError::Protocol),
        (24, WireError::Codec),
        (26, WireError::Coordination),
        (6, WireError::Epoch),
        (14, WireError::Epoch),
        (58, WireError::Mapping),
        (92, WireError::Peer),
    ] {
        let mut bytes = expected.clone();
        bytes[offset] ^= 1;
        let mut s = Session::new(&c, Member::new(0)).unwrap();
        assert_eq!(s.accept_handshake(&bytes), Err(failure));
        assert_eq!(s.accept_handshake(&expected), Err(WireError::SessionFailed));
    }
    let mut wrong_peer = Session::new(&c, Member::new(1)).unwrap();
    assert_eq!(wrong_peer.accept_handshake(&expected), Err(WireError::Peer));
    let mut early = Session::new(&c, Member::new(0)).unwrap();
    assert_eq!(early.decode(&frame(&[7])), Err(WireError::NotAdmitted));
    assert_eq!(
        early.accept_handshake(&expected),
        Err(WireError::SessionFailed)
    );
}
#[test]
fn malformed_frames_cannot_admit_routes_or_survive_failure() {
    let c = contract();
    let valid = frame(&[&[3][..], &[0; 33]].concat());
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
    for kind in [0, 13, 14, 255] {
        bad_frames.push(frame(&[kind]));
    }
    for offset in [5, 6, 14, 25, 26, 27] {
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
        assert_eq!(s.decode(&frame(&[7])), Err(WireError::SessionFailed));
    }
    let mut s = Session::new(&c, Member::new(1)).unwrap();
    preflight(&mut s, "beta");
    assert_eq!(s.decode(&valid), Err(WireError::Route));
}
#[test]
fn bounds_are_checked_without_size_driven_storage() {
    let c = contract();
    let mut s = Session::new(&c, Member::new(0)).unwrap();
    preflight(&mut s, "alpha");
    assert_eq!(s.decode(&[255, 255, 255, 255]), Err(WireError::Oversize));
    let mut s = Session::new(&c, Member::new(0)).unwrap();
    preflight(&mut s, "alpha");
    let oversized = frame(&[&[3][..], &[0; 29], &[0, 1, 0, 0]].concat());
    assert_eq!(s.decode(&oversized), Err(WireError::Oversize));
    let bytes = [0; MAX_PAYLOAD_BYTES + 1];
    let mut s = Session::new(&c, Member::new(0)).unwrap();
    preflight(&mut s, "alpha");
    assert_eq!(
        s.encode(
            &Message::PayloadToRti {
                route: Route::new(0),
                tag: WireTag::ZERO,
                payload: &bytes
            },
            &mut [0; 8]
        ),
        Err(WireError::Oversize)
    );
    let mut s = Session::new(&c, Member::new(0)).unwrap();
    preflight(&mut s, "alpha");
    assert_eq!(
        s.encode(&Message::Started, &mut [0; 21]),
        Err(WireError::Truncated)
    );
    let mut s = Session::new(&c, Member::new(0)).unwrap();
    preflight(&mut s, "alpha");
    let mut storage = [0; MAX_FRAME_BYTES];
    let n = s
        .encode(
            &Message::PayloadToRti {
                route: Route::new(0),
                tag: WireTag::ZERO,
                payload: &bytes[..MAX_PAYLOAD_BYTES],
            },
            &mut storage,
        )
        .unwrap();
    assert_eq!(n, MAX_FRAME_BYTES);
    assert!(s.decode(&storage).is_ok());
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
            Err(WireError::Mapping)
        ));
    }
    for kind in [6, 12] {
        let mut s = Session::new(&c, Member::new(0)).unwrap();
        preflight(&mut s, "alpha");
        assert!(s.decode(&frame(&[kind, 0, 0])).is_ok());
        assert_eq!(s.decode(&frame(&[7])), Err(WireError::SessionFailed));
    }
    for (kind, bytes, error) in [
        (6, vec![0, 1, 255], WireError::Invalid),
        (6, vec![4, 1], WireError::Oversize),
        (1, vec![0, 0, 0, 0, 0, 0, 0, 0, 2], WireError::Invalid),
    ] {
        let mut s = Session::new(&c, Member::new(0)).unwrap();
        preflight(&mut s, "alpha");
        assert_eq!(
            s.decode(&frame(&[&[kind][..], &bytes].concat())),
            Err(error)
        );
    }
}
