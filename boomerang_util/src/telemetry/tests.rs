use super::*;
use boomerang_runtime::{SchedulerLifecycle, SchedulerPhase};
use boomerang_telemetry::{
    CodecError, EncoderError, ExporterHealth, RecordGroup, SourceIdentity, SourceRole,
    TelemetryIdentity, TelemetryRecord, TelemetryValue, MAX_DATAGRAM_BYTES,
};
use std::{sync::Arc, time::Duration};
use tokio::net::UdpSocket;

fn identity() -> TelemetryIdentity<'static> {
    TelemetryIdentity {
        run_id: [1; 16],
        artifact_id: [2; 32],
        process_id: "worker-7",
        process_incarnation: 3,
        source: SourceIdentity {
            role: SourceRole::Federate,
            federate_id: Some("controller"),
            enclave_id: Some("plant"),
        },
    }
}

fn decode_record(input: &[u8]) -> TelemetryRecord<'_> {
    TelemetryRecord::decode(input, &mut [0; MAX_DATAGRAM_BYTES]).unwrap()
}

#[test]
fn source_samples_the_exposed_handle_using_its_observation_clock() {
    let mut source = TelemetrySource::new(identity());
    let observation = source.observation_handle();
    assert!(Arc::ptr_eq(&observation, &source.observation_handle()));
    observation.mark_running();
    observation.enter(SchedulerPhase::Reaction, source.origin);
    observation.record_completed_tag(source.origin + Duration::from_nanos(10));
    observation.add_processed_reactions(7);

    let mut output = [0; MAX_DATAGRAM_BYTES];
    let length = source.encode_scheduler(&mut output).unwrap().unwrap();
    let record = decode_record(&output[..length]);
    assert_eq!(record.run_id, [1; 16]);
    assert_eq!(record.artifact_id, [2; 32]);
    assert_eq!(record.process_id, "worker-7");
    assert_eq!(record.process_incarnation, 3);
    assert_eq!(record.source.federate_id, Some("controller"));
    assert_eq!(record.source.enclave_id, Some("plant"));
    assert_eq!(record.group_sequence, 0);
    assert!(record.sender_monotonic_ns >= record.observation_monotonic_ns);
    let TelemetryValue::Scheduler(sample) = record.value else {
        panic!("source did not emit scheduler observations");
    };
    assert_eq!(sample.lifecycle, SchedulerLifecycle::Running);
    assert_eq!(sample.current_phase, SchedulerPhase::Reaction);
    assert_eq!(sample.reaction_elapsed_ns, record.observation_monotonic_ns);
    assert_eq!(sample.completed_logical_tags, 1);
    assert_eq!(sample.last_logical_progress_ns, Some(10));
    assert_eq!(sample.processed_reactions, 7);

    let length = source.encode_exporter_health(&mut output).unwrap();
    let record = decode_record(&output[..length]);
    assert_eq!(record.group, RecordGroup::ExporterHealth);
    assert_eq!(record.group_sequence, 0);
    assert_eq!(
        record.value,
        TelemetryValue::ExporterHealth(ExporterHealth {
            publication_drops: 0,
            snapshot_misses: 0,
        })
    );
    let length = source.encode_scheduler(&mut output).unwrap().unwrap();
    assert_eq!(decode_record(&output[..length]).group_sequence, 1);
}

#[test]
fn source_encoding_respects_the_caller_buffer_without_consuming_sequence() {
    let mut source = TelemetrySource::new(identity());
    assert_eq!(
        source.encode_scheduler(&mut []),
        Err(EncoderError::Codec(CodecError::BufferTooSmall))
    );
    let mut output = [0; MAX_DATAGRAM_BYTES];
    let length = source.encode_scheduler(&mut output).unwrap().unwrap();
    assert_eq!(decode_record(&output[..length]).group_sequence, 0);
}

#[test]
fn exporter_rejects_zero_period_before_starting_thread() {
    let error = match TelemetryExporter::start(
        [identity()],
        "127.0.0.1:9".parse().unwrap(),
        Duration::ZERO,
    ) {
        Ok(_) => panic!("zero cadence must be rejected synchronously"),
        Err(error) => error,
    };

    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
}

#[tokio::test(flavor = "current_thread")]
async fn udp_worker_publishes_each_source_and_final_scheduler_state() {
    let receiver = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let mut sources = [TelemetrySource::new(identity()), {
        let mut second = identity();
        second.source.enclave_id = Some("sensor");
        TelemetrySource::new(second)
    }];
    let handles = sources.each_ref().map(TelemetrySource::observation_handle);
    for handle in &handles {
        handle.mark_running();
    }
    let mut received = Vec::new();
    let shutdown = async {
        for _ in 0..4 {
            let mut bytes = [0; MAX_DATAGRAM_BYTES];
            let length = receiver.recv(&mut bytes).await.unwrap();
            let record = decode_record(&bytes[..length]);
            received.push((
                record.source.enclave_id.unwrap().to_owned(),
                record.group,
                record.group_sequence,
                record.value,
            ));
        }
        handles[0].mark_stopped();
        handles[1].mark_failed();
    };
    tokio::time::timeout(
        Duration::from_secs(2),
        run_udp(
            &mut sources,
            receiver.local_addr().unwrap(),
            Duration::from_secs(60),
            shutdown,
        ),
    )
    .await
    .expect("worker did not publish or stop promptly")
    .unwrap();

    for expected_enclave in ["plant", "sensor"] {
        let (enclave, group, sequence, value) = received.remove(0);
        assert_eq!(enclave, expected_enclave);
        assert_eq!((group, sequence), (RecordGroup::Scheduler, 0));
        let TelemetryValue::Scheduler(sample) = value else {
            panic!("missing scheduler sample");
        };
        assert_eq!(sample.lifecycle, SchedulerLifecycle::Running);
        let (enclave, group, sequence, value) = received.remove(0);
        assert_eq!(enclave, expected_enclave);
        assert_eq!((group, sequence), (RecordGroup::ExporterHealth, 0));
        assert_eq!(
            value,
            TelemetryValue::ExporterHealth(ExporterHealth {
                publication_drops: 0,
                snapshot_misses: 0,
            })
        );
    }
    for (enclave, lifecycle) in [
        ("plant", SchedulerLifecycle::Stopped),
        ("sensor", SchedulerLifecycle::Failed),
    ] {
        for expected_group in [RecordGroup::Scheduler, RecordGroup::ExporterHealth] {
            let mut bytes = [0; MAX_DATAGRAM_BYTES];
            let length = tokio::time::timeout(Duration::from_secs(2), receiver.recv(&mut bytes))
                .await
                .expect("missing final publication")
                .unwrap();
            let record = decode_record(&bytes[..length]);
            assert_eq!(record.source.enclave_id, Some(enclave));
            assert_eq!((record.group, record.group_sequence), (expected_group, 1));
            if let TelemetryValue::Scheduler(sample) = record.value {
                assert_eq!(sample.lifecycle, lifecycle);
            }
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn udp_worker_rejects_zero_period_before_starting() {
    let mut sources = [TelemetrySource::new(identity())];
    let error = run_udp(
        &mut sources,
        "127.0.0.1:9".parse().unwrap(),
        Duration::ZERO,
        std::future::pending(),
    )
    .await
    .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
}

#[tokio::test(flavor = "current_thread")]
async fn udp_publication_failure_is_counted_without_retrying_or_failing_scheduler() {
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    socket.writable().await.unwrap();
    let mut sources = [TelemetrySource::new(identity())];
    let handle = sources[0].observation_handle();
    handle.mark_running();
    // An IPv4 socket cannot send to an IPv6 destination; both whole records are dropped.
    publish_round(
        &socket,
        "[::1]:9".parse().unwrap(),
        &mut sources,
        &mut [0; MAX_DATAGRAM_BYTES],
    );
    let mut output = [0; MAX_DATAGRAM_BYTES];
    let length = sources[0].encode_exporter_health(&mut output).unwrap();
    let record = decode_record(&output[..length]);
    assert_eq!(record.group_sequence, 1);
    assert_eq!(
        record.value,
        TelemetryValue::ExporterHealth(ExporterHealth {
            publication_drops: 2,
            snapshot_misses: 0,
        })
    );
    assert_eq!(
        handle
            .snapshot(std::time::Instant::now())
            .unwrap()
            .lifecycle,
        SchedulerLifecycle::Running
    );
    let length = sources[0].encode_scheduler(&mut output).unwrap().unwrap();
    assert_eq!(decode_record(&output[..length]).group_sequence, 1);
}
