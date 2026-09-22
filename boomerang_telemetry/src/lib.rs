#![no_std]

use serde::{Deserialize, Serialize};

/// Maximum encoded size of one telemetry record.
pub const MAX_DATAGRAM_BYTES: usize = 1_200;
const PROTOCOL_VERSION: u8 = 1;

/// An independently interpretable telemetry v1 record.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TelemetryRecord<'a> {
    /// Version of the portable internal record format.
    pub protocol_version: u8,
    /// Kind of values carried by this record.
    pub group: RecordGroup,
    /// Monotonically increasing sequence local to this source and group.
    pub group_sequence: u64,
    /// Identifier shared by every source in a generated run.
    pub run_id: [u8; 16],
    /// Identifier of the generated artifact being executed.
    pub artifact_id: [u8; 32],
    /// Stable operating-system process identity supplied by the adapter.
    pub process_id: &'a str,
    /// Process-start incarnation disambiguating reused process identifiers.
    pub process_incarnation: u64,
    /// Stable logical source identity supplied by the adapter.
    pub source: SourceIdentity<'a>,
    /// Monotonic nanoseconds at which this record was encoded.
    pub sender_monotonic_ns: u64,
    /// Monotonic nanoseconds at which the underlying observation was made.
    pub observation_monotonic_ns: u64,
    /// Values for the declared record group.
    pub value: TelemetryValue,
}

impl<'a> TelemetryRecord<'a> {
    /// Encodes this record canonically into caller-owned bounded storage.
    ///
    /// On success, the returned length selects the initialized prefix of `output`.
    ///
    /// # Errors
    ///
    /// Returns an error when the protocol version or record group is unsupported, the encoded
    /// record would exceed [`MAX_DATAGRAM_BYTES`], or `output` cannot hold the entire record.
    pub fn encode_into(&self, output: &mut [u8]) -> Result<usize, CodecError> {
        if self.protocol_version != PROTOCOL_VERSION {
            return Err(CodecError::UnsupportedVersion(self.protocol_version));
        }
        if !self.has_matching_group() {
            return Err(CodecError::GroupMismatch);
        }
        let length =
            postcard::experimental::serialized_size(self).map_err(|_| CodecError::Malformed)?;
        if length > MAX_DATAGRAM_BYTES {
            return Err(CodecError::Oversize);
        }
        let encoded = postcard::to_slice(
            self,
            output.get_mut(..length).ok_or(CodecError::BufferTooSmall)?,
        )
        .map_err(|_| CodecError::Malformed)?;
        Ok(encoded.len())
    }

    /// Decodes a bounded canonical record borrowing stable text from `input`.
    ///
    /// `scratch` is temporary canonicalization storage and must be at least as long as `input`.
    /// The returned process, Federate, and Enclave strings borrow from `input`, not `scratch`.
    ///
    /// # Errors
    ///
    /// Returns an error for oversized, malformed, trailing, noncanonical, unsupported-version,
    /// or group-mismatched input, or when `scratch` is shorter than `input`.
    pub fn decode(input: &'a [u8], scratch: &mut [u8]) -> Result<Self, CodecError> {
        if input.len() > MAX_DATAGRAM_BYTES {
            return Err(CodecError::Oversize);
        }
        let scratch = scratch
            .get_mut(..input.len())
            .ok_or(CodecError::BufferTooSmall)?;
        let (record, remaining) = postcard::take_from_bytes::<TelemetryRecord<'a>>(input)
            .map_err(|_| CodecError::Malformed)?;
        if !remaining.is_empty() {
            return Err(CodecError::TrailingData);
        }
        if record.protocol_version != PROTOCOL_VERSION {
            return Err(CodecError::UnsupportedVersion(record.protocol_version));
        }
        if !record.has_matching_group() {
            return Err(CodecError::GroupMismatch);
        }
        let canonical = postcard::to_slice(&record, scratch).map_err(|_| CodecError::Malformed)?;
        if canonical != input {
            return Err(CodecError::NonCanonical);
        }
        Ok(record)
    }

    const fn has_matching_group(&self) -> bool {
        matches!(
            (self.group, self.value),
            (RecordGroup::Scheduler, TelemetryValue::Scheduler(_))
                | (
                    RecordGroup::ExporterHealth,
                    TelemetryValue::ExporterHealth(_)
                )
        )
    }
}

/// Stable logical identity of the source process.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceIdentity<'a> {
    /// Role that produced the record.
    pub role: SourceRole,
    /// Stable Federate identifier, when the source belongs to a Federate.
    pub federate_id: Option<&'a str>,
    /// Stable Enclave identifier, when the source belongs to an Enclave.
    pub enclave_id: Option<&'a str>,
}

/// Portable source role without runtime dense indexes.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[repr(u8)]
pub enum SourceRole {
    /// A scheduler outside a Federate deployment.
    Standalone = 0,
    /// A scheduler hosted by a named Federate.
    Federate = 1,
    /// A central RTI or comparable coordination source.
    CentralRti = 2,
}

/// Independent telemetry value group.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[repr(u8)]
pub enum RecordGroup {
    /// Absolute scheduler observation values.
    Scheduler = 0,
    /// Exporter publication and observation-health counters.
    ExporterHealth = 1,
}

/// Absolute scheduler counters, gauges, peaks, lifecycle, and phase values.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SchedulerSample {
    /// Adapter-defined scheduler lifecycle code; this portable core assigns no code meanings.
    pub lifecycle: u8,
    /// Adapter-defined current scheduler phase code; this portable core assigns no code meanings.
    pub current_phase: u8,
    /// Monotonic nanoseconds at which the current phase began.
    pub current_phase_started_ns: u64,
    /// Cumulative nanoseconds spent in user reaction callbacks.
    pub reaction_elapsed_ns: u64,
    /// Cumulative nanoseconds spent in framework work.
    pub framework_elapsed_ns: u64,
    /// Cumulative nanoseconds waiting for physical time.
    pub physical_wait_elapsed_ns: u64,
    /// Cumulative nanoseconds waiting for external input.
    pub external_wait_elapsed_ns: u64,
    /// Cumulative nanoseconds waiting for Federate coordination.
    pub coordination_wait_elapsed_ns: u64,
    /// Cumulative scheduler tag-processing steps.
    pub processed_tags: u64,
    /// Cumulative enabled reaction callbacks selected for invocation.
    pub processed_reactions: u64,
    /// Cumulative asynchronous scheduler events handled.
    pub processed_events: u64,
    /// Cumulative present-port observations.
    pub set_ports: u64,
    /// Cumulative actions requested by reaction outcomes.
    pub scheduled_actions: u64,
    /// Event-queue occupancy at the observation point.
    pub event_queue_occupancy: u64,
    /// Event-queue slots reserved at the observation point.
    pub event_queue_reserved_capacity: u64,
    /// Runtime-enforced queue limit, if one exists.
    pub event_queue_enforced_limit: Option<u64>,
    /// Largest event-queue occupancy since observation started.
    pub event_queue_peak_occupancy: u64,
    /// Number of completed logical tags.
    pub completed_logical_tags: u64,
    /// Monotonic nanoseconds of the latest logical progress, if available.
    pub last_logical_progress_ns: Option<u64>,
}

/// Absolute health counters measured by a future exporter adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ExporterHealth {
    /// Records dropped before publication.
    pub publication_drops: u64,
    /// Coherent observation snapshots unavailable to the exporter.
    pub snapshot_misses: u64,
}

/// Values carried by a telemetry record.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TelemetryValue {
    /// Scheduler observation values.
    Scheduler(SchedulerSample),
    /// Exporter-health values.
    ExporterHealth(ExporterHealth),
}

/// Shared immutable header values for records emitted by one telemetry source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TelemetryIdentity<'a> {
    /// Identifier shared by every source in a generated run.
    pub run_id: [u8; 16],
    /// Identifier of the generated artifact being executed.
    pub artifact_id: [u8; 32],
    /// Stable operating-system process identity supplied by the adapter.
    pub process_id: &'a str,
    /// Process-start incarnation disambiguating reused process identifiers.
    pub process_incarnation: u64,
    /// Stable logical source identity supplied by the adapter.
    pub source: SourceIdentity<'a>,
}

/// Stateful, caller-buffered encoder for one portable telemetry source.
///
/// The encoder deliberately owns no clock, transport, queue, or runtime observation state.
pub struct TelemetryEncoder<'a> {
    identity: TelemetryIdentity<'a>,
    scheduler_next_sequence: Option<u64>,
    exporter_health_next_sequence: Option<u64>,
    exporter_health: ExporterHealth,
}

impl<'a> TelemetryEncoder<'a> {
    /// Starts a source encoder with independent record-group sequences at zero.
    #[must_use]
    pub const fn new(identity: TelemetryIdentity<'a>) -> Self {
        Self {
            identity,
            scheduler_next_sequence: Some(0),
            exporter_health_next_sequence: Some(0),
            exporter_health: ExporterHealth {
                publication_drops: 0,
                snapshot_misses: 0,
            },
        }
    }

    /// Records a publication failure after its sequence has already been consumed.
    pub fn note_publication_drop(&mut self) {
        self.exporter_health.publication_drops =
            self.exporter_health.publication_drops.saturating_add(1);
    }

    /// Records an unavailable coherent observation snapshot.
    pub fn note_snapshot_miss(&mut self) {
        self.exporter_health.snapshot_misses =
            self.exporter_health.snapshot_misses.saturating_add(1);
    }

    /// Encodes one scheduler sample into caller-owned bounded storage.
    ///
    /// # Errors
    ///
    /// Returns [`EncoderError::SequenceExhausted`] after this group's final sequence, or wraps a
    /// bounded codec failure without advancing the sequence.
    pub fn encode_scheduler(
        &mut self,
        sender_monotonic_ns: u64,
        observation_monotonic_ns: u64,
        sample: SchedulerSample,
        output: &mut [u8],
    ) -> Result<usize, EncoderError> {
        self.encode(
            RecordGroup::Scheduler,
            sender_monotonic_ns,
            observation_monotonic_ns,
            TelemetryValue::Scheduler(sample),
            output,
        )
    }

    /// Encodes cumulative exporter-health counters into caller-owned bounded storage.
    ///
    /// # Errors
    ///
    /// Returns [`EncoderError::SequenceExhausted`] after this group's final sequence, or wraps a
    /// bounded codec failure without advancing the sequence.
    pub fn encode_exporter_health(
        &mut self,
        sender_monotonic_ns: u64,
        observation_monotonic_ns: u64,
        output: &mut [u8],
    ) -> Result<usize, EncoderError> {
        self.encode(
            RecordGroup::ExporterHealth,
            sender_monotonic_ns,
            observation_monotonic_ns,
            TelemetryValue::ExporterHealth(self.exporter_health),
            output,
        )
    }

    fn encode(
        &mut self,
        group: RecordGroup,
        sender_monotonic_ns: u64,
        observation_monotonic_ns: u64,
        value: TelemetryValue,
        output: &mut [u8],
    ) -> Result<usize, EncoderError> {
        let group_sequence = self.next_sequence(group)?;
        let record = TelemetryRecord {
            protocol_version: PROTOCOL_VERSION,
            group,
            group_sequence,
            run_id: self.identity.run_id,
            artifact_id: self.identity.artifact_id,
            process_id: self.identity.process_id,
            process_incarnation: self.identity.process_incarnation,
            source: self.identity.source,
            sender_monotonic_ns,
            observation_monotonic_ns,
            value,
        };
        let length = record.encode_into(output).map_err(EncoderError::Codec)?;
        self.advance_sequence(group, group_sequence);
        Ok(length)
    }

    fn next_sequence(&self, group: RecordGroup) -> Result<u64, EncoderError> {
        match group {
            RecordGroup::Scheduler => self.scheduler_next_sequence,
            RecordGroup::ExporterHealth => self.exporter_health_next_sequence,
        }
        .ok_or(EncoderError::SequenceExhausted(group))
    }

    fn advance_sequence(&mut self, group: RecordGroup, sequence: u64) {
        match group {
            RecordGroup::Scheduler => self.scheduler_next_sequence = sequence.checked_add(1),
            RecordGroup::ExporterHealth => {
                self.exporter_health_next_sequence = sequence.checked_add(1);
            }
        }
    }
}

/// Stateful encoder failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EncoderError {
    /// This record group successfully emitted `u64::MAX` and cannot wrap.
    SequenceExhausted(RecordGroup),
    /// Existing bounded codec failure while constructing a record.
    Codec(CodecError),
}

/// Bounded codec failure without allocator- or transport-specific details.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodecError {
    /// A record or input exceeds the datagram ceiling.
    Oversize,
    /// Caller-provided output or scratch storage is too short.
    BufferTooSmall,
    /// Bytes cannot be decoded as a telemetry record.
    Malformed,
    /// Bytes remain after a decoded record.
    TrailingData,
    /// The input is a noncanonical representation of the decoded record.
    NonCanonical,
    /// The record protocol version is not supported.
    UnsupportedVersion(u8),
    /// Record group and value variant disagree.
    GroupMismatch,
}

#[cfg(test)]
mod tests {
    use super::*;

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
            value: TelemetryValue::Scheduler(SchedulerSample {
                lifecycle: 1,
                current_phase: 2,
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

    fn scheduler_sample() -> SchedulerSample {
        match scheduler_record().value {
            TelemetryValue::Scheduler(sample) => sample,
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
    fn codec_rejects_malformed_and_unknown_version_records() {
        let mut unsupported = scheduler_record();
        unsupported.protocol_version = 2;
        let mut output = [0; MAX_DATAGRAM_BYTES];
        assert_eq!(
            unsupported.encode_into(&mut output),
            Err(CodecError::UnsupportedVersion(2))
        );

        let mut scratch = [0; MAX_DATAGRAM_BYTES];
        assert_eq!(
            TelemetryRecord::decode(&[0xff], &mut scratch),
            Err(CodecError::Malformed)
        );

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

        let scheduler_length = encoder
            .encode_scheduler(1_000, 900, scheduler_sample(), &mut output)
            .unwrap();
        let scheduler = decode_record(&output[..scheduler_length]);
        assert_eq!(scheduler.group, RecordGroup::Scheduler);
        assert_eq!(scheduler.group_sequence, 0);
        assert_eq!(scheduler.run_id, [1; 16]);
        assert_eq!(scheduler.source.federate_id, Some("controller"));

        let health_length = encoder
            .encode_exporter_health(1_001, 901, &mut output)
            .unwrap();
        let health = decode_record(&output[..health_length]);
        assert_eq!(health.group, RecordGroup::ExporterHealth);
        assert_eq!(health.group_sequence, 0);

        let scheduler_length = encoder
            .encode_scheduler(1_002, 902, scheduler_sample(), &mut output)
            .unwrap();
        let scheduler = decode_record(&output[..scheduler_length]);
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
                snapshot_misses: u64::MAX,
            })
        );
    }

    #[test]
    fn telemetry_encoder_exhausted_scheduler_does_not_block_health() {
        let mut encoder = TelemetryEncoder::new(telemetry_identity());
        encoder.scheduler_next_sequence = Some(u64::MAX);
        let mut output = [0; MAX_DATAGRAM_BYTES];

        let last_scheduler_length = encoder
            .encode_scheduler(1_000, 900, scheduler_sample(), &mut output)
            .unwrap();
        assert_eq!(
            decode_record(&output[..last_scheduler_length]).group_sequence,
            u64::MAX
        );

        let unchanged = [0xa5; MAX_DATAGRAM_BYTES];
        output = unchanged;
        assert_eq!(
            encoder.encode_scheduler(1_001, 901, scheduler_sample(), &mut output),
            Err(EncoderError::SequenceExhausted(RecordGroup::Scheduler))
        );
        assert_eq!(output, unchanged);

        let health_length = encoder
            .encode_exporter_health(1_002, 902, &mut output)
            .unwrap();
        let health = decode_record(&output[..health_length]);
        assert_eq!(health.group, RecordGroup::ExporterHealth);
        assert_eq!(health.group_sequence, 0);
    }
}
