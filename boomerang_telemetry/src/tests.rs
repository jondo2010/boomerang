use super::*;
use boomerang_runtime::{ObservationSnapshot, SchedulerLifecycle, SchedulerPhase};

fn scheduler_record<'a>() -> TelemetryRecord<'a> {
    TelemetryRecord {
        protocol_version: 1,
        group: RecordGroup::Scheduler,
        group_sequence: 9,
        run_id: [1; 16],
        artifact_id: [2; 32],
        process_id: "scheduler-7",
        process_incarnation: 3,
        source: SourceIdentity {
            role: SourceRole::Federate,
            federate_id: Some("controller"),
            enclave_id: Some("plant"),
        },
        sender_monotonic_ns: 1_000,
        observation_monotonic_ns: 900,
        value: TelemetryValue::Scheduler(ObservationSnapshot {
            lifecycle: SchedulerLifecycle::Running,
            current_phase: SchedulerPhase::Framework,
            current_phase_started_ns: 800,
            reaction_elapsed_ns: 10,
            framework_elapsed_ns: 20,
            physical_wait_elapsed_ns: 30,
            external_wait_elapsed_ns: 40,
            coordination_wait_elapsed_ns: 50,
            processed_tags: 60,
            processed_reactions: 70,
            processed_events: 80,
            set_ports: 90,
            scheduled_actions: 100,
            event_queue_occupancy: 11,
            event_queue_reserved_capacity: 12,
            event_queue_enforced_limit: None,
            event_queue_peak_occupancy: 13,
            completed_logical_tags: 14,
            last_logical_progress_ns: None,
        }),
    }
}

fn telemetry_identity() -> TelemetryIdentity<'static> {
    TelemetryIdentity {
        run_id: [1; 16],
        artifact_id: [2; 32],
        process_id: "scheduler-7",
        process_incarnation: 3,
        source: SourceIdentity {
            role: SourceRole::Federate,
            federate_id: Some("controller"),
            enclave_id: Some("plant"),
        },
    }
}
fn scheduler_sample() -> ObservationSnapshot {
    match scheduler_record().value {
        TelemetryValue::Scheduler(value) => value,
        TelemetryValue::ExporterHealth(_) => unreachable!(),
    }
}
fn decode_record(input: &[u8]) -> TelemetryRecord<'_> {
    let mut scratch = [0; MAX_DATAGRAM_BYTES];
    TelemetryRecord::decode(input, &mut scratch).unwrap()
}

#[test]
fn scheduler_round_trip_preserves_stable_identity_and_none() {
    let record = scheduler_record();
    let mut encoded = [0; MAX_DATAGRAM_BYTES];
    let length = record.encode_into(&mut encoded).unwrap();
    let mut scratch = [0; MAX_DATAGRAM_BYTES];
    assert_eq!(
        TelemetryRecord::decode(&encoded[..length], &mut scratch),
        Ok(record)
    );
}
#[test]
fn codec_rejects_undersized_buffers_and_oversized_input() {
    let record = scheduler_record();
    let mut full = [0; MAX_DATAGRAM_BYTES];
    let length = record.encode_into(&mut full).unwrap();
    let mut exact = [0; MAX_DATAGRAM_BYTES];
    assert_eq!(record.encode_into(&mut exact[..length]), Ok(length));
    assert_eq!(
        record.encode_into(&mut exact[..length - 1]),
        Err(CodecError::BufferTooSmall)
    );
    assert_eq!(
        TelemetryRecord::decode(&[0; MAX_DATAGRAM_BYTES + 1], &mut exact),
        Err(CodecError::Oversize)
    );
}
#[test]
fn codec_retains_postcard_errors_and_rejects_unknown_versions() {
    let mut unsupported = scheduler_record();
    unsupported.protocol_version = 2;
    let mut output = [0; MAX_DATAGRAM_BYTES];
    assert_eq!(
        unsupported.encode_into(&mut output),
        Err(CodecError::UnsupportedVersion(2))
    );
    let mut scratch = [0; MAX_DATAGRAM_BYTES];
    assert!(matches!(
        TelemetryRecord::decode(&[0xff], &mut scratch),
        Err(CodecError::Codec(_))
    ));
    let record = scheduler_record();
    let mut encoded = [0; MAX_DATAGRAM_BYTES];
    let length = record.encode_into(&mut encoded).unwrap();
    encoded[0] = 2;
    assert_eq!(
        TelemetryRecord::decode(&encoded[..length], &mut scratch),
        Err(CodecError::UnsupportedVersion(2))
    );
}
#[test]
fn telemetry_encoder_sequences_scheduler_health_scheduler_independently() {
    let mut encoder = TelemetryEncoder::new(telemetry_identity());
    let mut output = [0; MAX_DATAGRAM_BYTES];
    let length = encoder
        .encode_scheduler(1_000, 900, scheduler_sample(), &mut output)
        .unwrap();
    let scheduler = decode_record(&output[..length]);
    assert_eq!(
        (scheduler.group, scheduler.group_sequence),
        (RecordGroup::Scheduler, 0)
    );
    assert_eq!(scheduler.run_id, [1; 16]);
    assert_eq!(scheduler.source.federate_id, Some("controller"));
    let length = encoder
        .encode_exporter_health(1_001, 901, &mut output)
        .unwrap();
    let health = decode_record(&output[..length]);
    assert_eq!(
        (health.group, health.group_sequence),
        (RecordGroup::ExporterHealth, 0)
    );
    let length = encoder
        .encode_scheduler(1_002, 902, scheduler_sample(), &mut output)
        .unwrap();
    let scheduler = decode_record(&output[..length]);
    assert_eq!(scheduler.group, RecordGroup::Scheduler);
    assert_eq!(scheduler.group_sequence, 1);
}
#[test]
fn telemetry_encoder_retries_short_buffer_at_same_scheduler_sequence() {
    let mut encoder = TelemetryEncoder::new(telemetry_identity());
    let mut short = [0xa5; 1];
    assert_eq!(
        encoder.encode_scheduler(1_000, 900, scheduler_sample(), &mut short),
        Err(EncoderError::Codec(CodecError::BufferTooSmall))
    );
    assert_eq!(short, [0xa5; 1]);
    let mut output = [0; MAX_DATAGRAM_BYTES];
    let length = encoder
        .encode_scheduler(1_000, 900, scheduler_sample(), &mut output)
        .unwrap();
    assert_eq!(decode_record(&output[..length]).group_sequence, 0);
}
#[test]
fn telemetry_encoder_encodes_saturating_health_counters() {
    let mut encoder = TelemetryEncoder::new(telemetry_identity());
    encoder.exporter_health = ExporterHealth {
        publication_drops: u64::MAX - 1,
        snapshot_misses: u64::MAX - 1,
    };
    encoder.note_publication_drop();
    encoder.note_publication_drop();
    encoder.note_snapshot_miss();
    encoder.note_snapshot_miss();
    let mut output = [0; MAX_DATAGRAM_BYTES];
    let length = encoder
        .encode_exporter_health(1_000, 900, &mut output)
        .unwrap();
    let health = decode_record(&output[..length]);
    assert_eq!(health.group_sequence, 0);
    assert_eq!(
        health.value,
        TelemetryValue::ExporterHealth(ExporterHealth {
            publication_drops: u64::MAX,
            snapshot_misses: u64::MAX
        })
    );
}
#[test]
fn telemetry_encoder_exhausted_scheduler_does_not_block_health() {
    let mut encoder = TelemetryEncoder::new(telemetry_identity());
    encoder.scheduler_next_sequence = Some(u64::MAX);
    let mut output = [0; MAX_DATAGRAM_BYTES];
    let length = encoder
        .encode_scheduler(1_000, 900, scheduler_sample(), &mut output)
        .unwrap();
    assert_eq!(decode_record(&output[..length]).group_sequence, u64::MAX);
    let unchanged = [0xa5; MAX_DATAGRAM_BYTES];
    output = unchanged;
    assert_eq!(
        encoder.encode_scheduler(1_001, 901, scheduler_sample(), &mut output),
        Err(EncoderError::SequenceExhausted(RecordGroup::Scheduler))
    );
    assert_eq!(output, unchanged);
    let length = encoder
        .encode_exporter_health(1_002, 902, &mut output)
        .unwrap();
    let health = decode_record(&output[..length]);
    assert_eq!(
        (health.group, health.group_sequence),
        (RecordGroup::ExporterHealth, 0)
    );
}

#[test]
fn errors_implement_display_and_convert_codec_failures() {
    let error = EncoderError::from(CodecError::BufferTooSmall);
    fn accepts_display(_: &impl core::fmt::Display) {}
    fn accepts_error(_: &impl core::error::Error) {}

    accepts_display(&error);
    accepts_error(&error);
}
