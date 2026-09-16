// Appended to a disposable copy of an actual generated RTI artifact for executable verification.

/// Generated tables, metadata, bounded value codec, framing, and closed admission agree.
#[test]
fn generated_wire_contract_conformance() {
    use boomerang_federated::{protocol::WireTag, wire::*};
    let hex = |bytes: [u8; 32]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    assert_eq!(
        hex(WIRE_COORDINATION_FINGERPRINT.bytes()),
        std::env::var("EXPECTED_COORDINATION").unwrap()
    );
    assert_eq!(
        hex(WIRE_MAPPING),
        std::env::var("EXPECTED_MAPPING").unwrap()
    );
    let _view = RtiImageView::new(COORDINATION_IMAGE, COORDINATION_MEMBERS).unwrap();
    let (route_key, route) = COORDINATION_IMAGE.routes().iter().next().unwrap();
    let member = route.source();
    let contract = wire_contract();
    let mut session = Session::new(&contract, member).unwrap();
    let mut frame = [0; 512];
    let mut handshake = wire_handshake(member).unwrap();
    let size = encode_handshake(&handshake, &mut frame).unwrap();
    assert_eq!(session.accept_handshake(&frame[..size]).unwrap(), member);
    assert_eq!(
        WirePayloadCodec::<u32>::MAX_ENCODED_BYTES,
        MAX_PAYLOAD_BYTES
    );
    let mut value_bytes = [0; 5];
    let size = WirePayloadCodec::<u32>::encode(&300, &mut value_bytes).unwrap();
    assert_eq!(&value_bytes[..size], &[0xac, 0x02]);
    let message = Message::Request(Request::Payload {
        route: route_key,
        tag: WireTag::finite(1_000_000, 7),
        payload: &value_bytes[..size],
    });
    let size = session.encode(&message, &mut frame).unwrap();
    let decoded = session.decode(&frame[..size]).unwrap();
    assert_eq!(decoded, message);
    let Message::Request(Request::Payload { payload, .. }) = decoded else {
        panic!("wrong message")
    };
    assert_eq!(
        WirePayloadCodec::<u32>::decode(payload, &mut [0; 5]).unwrap(),
        300
    );
    assert!(WirePayloadCodec::<u32>::decode(&[0xac, 0x02, 0], &mut [0; 5]).is_err());
    let mut rejected = Session::new(&contract, member).unwrap();
    handshake.mapping[0] ^= 1;
    let size = encode_handshake(&handshake, &mut frame).unwrap();
    assert_eq!(
        rejected.accept_handshake(&frame[..size]),
        Err(WireError::Admission(AdmissionError::Mapping))
    );
    assert!(rejected.decode(&frame[..size]).is_err());
}

/// The generated resource contract bounds actual RTI admission before payload forwarding.
#[test]
fn generated_rti_enforces_in_transit_capacity() {
    use boomerang_central_rti::compiled::{CompiledRti, RtiReply, RtiRequest};
    use boomerang_federated::protocol::WireTag;
    let view = RtiImageView::new(COORDINATION_IMAGE, COORDINATION_MEMBERS).unwrap();
    let mut rti = CompiledRti::from_image(view, COORDINATION_IDENTITY).unwrap();
    for (member, _) in COORDINATION_IMAGE.members().iter() {
        rti.handle(member, RtiRequest::Hello { identity: COORDINATION_IDENTITY });
    }
    let (key, route) = COORDINATION_IMAGE.routes().iter().next().unwrap();
    let capacity = COORDINATION_IMAGE.members()[route.target()].in_transit_capacity();
    assert!(capacity > 0);
    rti.handle(route.source(), RtiRequest::Publish { revision: 1, next_event: Some(WireTag::ZERO) });
    for index in 0..=capacity {
        let replies = rti.handle(route.source(), RtiRequest::Payload {
            route: key,
            tag: WireTag::finite(i128::from(route.delay_nanos()) + i128::from(index), 0),
            payload: vec![42],
        });
        if index < capacity {
            assert!(replies.iter().any(|delivery| matches!(delivery.reply, RtiReply::Payload { .. })));
        } else {
            assert!(!replies.is_empty());
            assert!(replies.iter().all(|delivery| matches!(&delivery.reply,
                RtiReply::Failed { message } if message.contains("in-transit tag capacity"))));
        }
    }
}
