use std::{
    cmp::Ordering,
    collections::{BTreeMap, VecDeque},
    time::Duration,
};

use boomerang_telemetry::{
    CodecError, ExporterHealth, ObservationSnapshot, RecordGroup, SourceRole, TelemetryRecord,
    TelemetryValue, MAX_DATAGRAM_BYTES,
};
use serde::Serialize;

/// Limits for receiver-owned state. Zero disables registration or history retention.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReceiverConfig {
    /// Maximum distinct complete source identities retained by the receiver.
    pub max_sources: usize,
    /// Maximum scheduler samples retained for each registered source.
    pub history_capacity: usize,
    /// Aggregate bytes of copied process, Federate, and Enclave identity text.
    pub max_metadata_bytes: usize,
}

impl Default for ReceiverConfig {
    fn default() -> Self {
        Self {
            max_sources: 64,
            history_capacity: 120,
            max_metadata_bytes: 64 * 1024,
        }
    }
}

/// Whether a datagram changed a group's current observation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IngestOutcome {
    /// The datagram advanced its group state and was retained.
    Accepted,
    /// The datagram could not be decoded as a valid canonical telemetry record.
    Malformed(CodecError),
    /// A new source exceeded the configured source-count limit.
    SourceLimitRejected,
    /// A new source exceeded the configured copied-metadata byte limit.
    MetadataLimitRejected,
    /// The record sequence repeated or moved backwards within its group.
    StaleOrReordered,
}

/// Mutually exclusive codec rejection categories.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct MalformedCounters {
    /// Inputs larger than the telemetry datagram ceiling.
    pub oversize: u64,
    /// Concrete Postcard codec failures.
    pub codec: u64,
    /// Inputs containing bytes after a decoded record.
    pub trailing_data: u64,
    /// Decodable inputs that are not in canonical Postcard form.
    pub noncanonical: u64,
    /// Inputs with an unsupported telemetry envelope version.
    pub unsupported_version: u64,
    /// Inputs whose record group disagrees with the value variant.
    pub group_mismatch: u64,
    /// Inputs that could not use the caller-provided decode scratch storage.
    pub buffer_too_small: u64,
}

impl MalformedCounters {
    /// Returns the saturating sum of every malformed-input category.
    pub fn total(&self) -> u64 {
        [
            self.oversize,
            self.codec,
            self.trailing_data,
            self.noncanonical,
            self.unsupported_version,
            self.group_mismatch,
            self.buffer_too_small,
        ]
        .into_iter()
        .fold(0, u64::saturating_add)
    }

    fn record(&mut self, error: &CodecError) {
        let counter = match error {
            CodecError::Oversize => &mut self.oversize,
            CodecError::Codec(_) => &mut self.codec,
            CodecError::TrailingData => &mut self.trailing_data,
            CodecError::NonCanonical => &mut self.noncanonical,
            CodecError::UnsupportedVersion(_) => &mut self.unsupported_version,
            CodecError::GroupMismatch => &mut self.group_mismatch,
            CodecError::BufferTooSmall => &mut self.buffer_too_small,
        };
        *counter = counter.saturating_add(1);
    }
}

/// Aggregate receiver statistics. Counts saturate rather than wrap at `u64::MAX`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct ReceiverCounters {
    /// Records accepted into a source's group state.
    pub accepted: u64,
    /// Rejections classified by telemetry codec failure.
    pub malformed: MalformedCounters,
    /// New sources rejected because the registry is full.
    pub source_limit_rejected: u64,
    /// New sources rejected because copied identity metadata exceeds its budget.
    pub metadata_limit_rejected: u64,
    /// Records rejected because their group sequence did not advance.
    pub stale_or_reordered: u64,
    /// Missing sequence values inferred from accepted forward sequence gaps.
    pub skipped_sequences: u64,
    /// Accepted scheduler intervals with one or more discontinuity conditions.
    pub discontinuities: u64,
    /// Scheduler samples discarded because bounded history could not retain them.
    pub history_evictions: u64,
}

impl ReceiverCounters {
    /// Returns the saturating sum of all rejected input categories.
    pub fn rejected(&self) -> u64 {
        self.malformed
            .total()
            .saturating_add(self.source_limit_rejected)
            .saturating_add(self.metadata_limit_rejected)
            .saturating_add(self.stale_or_reordered)
    }
}

/// Owned identity, without runtime dense keys or inferred process ordinals.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SourceIdentitySnapshot {
    /// Run identifier shared by sources in one generated deployment run.
    pub run_id: [u8; 16],
    /// Fingerprint of the generated artifact that emitted the record.
    pub artifact_id: [u8; 32],
    /// Stable operating-system process identity copied from the record.
    pub process_id: String,
    /// Process-start incarnation distinguishing reused process identities.
    pub process_incarnation: u64,
    /// Logical role of the telemetry producer.
    pub role: SourceRole,
    /// Federate identity when the source belongs to a Federate.
    pub federate_id: Option<String>,
    /// Enclave identity when the source belongs to an Enclave.
    pub enclave_id: Option<String>,
}

#[derive(Debug, Eq, PartialEq)]
struct SourceKey(SourceIdentitySnapshot);

impl SourceKey {
    fn matches(&self, record: &TelemetryRecord<'_>) -> bool {
        let identity = &self.0;
        identity.run_id == record.run_id
            && identity.artifact_id == record.artifact_id
            && identity.process_id == record.process_id
            && identity.process_incarnation == record.process_incarnation
            && identity.role == record.source.role
            && identity.federate_id.as_deref() == record.source.federate_id
            && identity.enclave_id.as_deref() == record.source.enclave_id
    }

    fn copy_from(record: &TelemetryRecord<'_>) -> Self {
        Self(SourceIdentitySnapshot {
            run_id: record.run_id,
            artifact_id: record.artifact_id,
            process_id: record.process_id.to_owned(),
            process_incarnation: record.process_incarnation,
            role: record.source.role,
            federate_id: record.source.federate_id.map(str::to_owned),
            enclave_id: record.source.enclave_id.map(str::to_owned),
        })
    }
}

impl Ord for SourceKey {
    fn cmp(&self, other: &Self) -> Ordering {
        let left = &self.0;
        let right = &other.0;
        (
            left.run_id,
            left.artifact_id,
            &left.process_id,
            left.process_incarnation,
        )
            .cmp(&(
                right.run_id,
                right.artifact_id,
                &right.process_id,
                right.process_incarnation,
            ))
            .then_with(|| match (left.role, right.role) {
                (a, b) if a == b => Ordering::Equal,
                (SourceRole::Standalone, _) | (SourceRole::Federate, SourceRole::CentralRti) => {
                    Ordering::Less
                }
                _ => Ordering::Greater,
            })
            .then_with(|| left.federate_id.cmp(&right.federate_id))
            .then_with(|| left.enclave_id.cmp(&right.enclave_id))
    }
}

impl PartialOrd for SourceKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Group-local delivery state; rejected input never refreshes `fresh_at`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct SequenceSnapshot {
    /// Most recent accepted sequence in this group, if any.
    pub latest: Option<u64>,
    /// Sequence values skipped by accepted forward gaps.
    pub skipped: u64,
    /// Repeated or lower sequences rejected for this group.
    pub stale_or_reordered: u64,
    /// Accepted scheduler intervals that made rates unavailable.
    pub discontinuities: u64,
    /// Receipt time of the last accepted record on the receiver's clock.
    pub fresh_at: Option<Duration>,
    /// Time since `fresh_at`, unavailable if snapshot `now` precedes receipt.
    pub age: Option<Duration>,
}

impl SequenceSnapshot {
    fn accept(
        &mut self,
        sequence: u64,
        received_at: Duration,
        counters: &mut ReceiverCounters,
    ) -> bool {
        if let Some(previous) = self.latest {
            if sequence <= previous {
                self.stale_or_reordered = self.stale_or_reordered.saturating_add(1);
                counters.stale_or_reordered = counters.stale_or_reordered.saturating_add(1);
                return false;
            }
            let skipped = sequence - previous - 1;
            self.skipped = self.skipped.saturating_add(skipped);
            counters.skipped_sequences = counters.skipped_sequences.saturating_add(skipped);
        }
        self.latest = Some(sequence);
        self.fresh_at = Some(received_at);
        true
    }

    fn at(self, now: Duration) -> Self {
        Self {
            age: self.fresh_at.and_then(|at| now.checked_sub(at)),
            ..self
        }
    }
}

// The field list couples each cumulative payload field to its named rate. Gauges
// are intentionally absent. Keep this exhaustive as the internal payload evolves.
macro_rules! scheduler_rates {
    ($($counter:ident => $rate:ident),+ $(,)?) => {
        /// Integer per-second rates, rounded down. A missing baseline or a
        /// discontinuity makes every rate unavailable for that sample. An
        /// individual rate above `u64::MAX` is also unavailable, never wrapped.
        #[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
        pub struct SchedulerRates {
            $(#[doc = concat!("Per-second rate for [`ObservationSnapshot::", stringify!($counter), "`].")]
            pub $rate: Option<u64>,)+
        }

        impl SchedulerRates {
            fn between(previous: &SchedulerSample, observation_ns: u64, raw: &ObservationSnapshot) -> (Self, bool) {
                let Some(elapsed) = observation_ns.checked_sub(previous.observation_monotonic_ns).filter(|delta| *delta > 0) else {
                    return (Self::default(), true);
                };
                if $(raw.$counter < previous.raw.$counter
                    || raw.$counter == u64::MAX || previous.raw.$counter == u64::MAX)||+ {
                    return (Self::default(), true);
                }
                (Self { $($rate: u64::try_from(
                    u128::from(raw.$counter - previous.raw.$counter) * 1_000_000_000 / u128::from(elapsed)
                ).ok(),)+ }, false)
            }
        }
    };
}

scheduler_rates! {
    reaction_elapsed_ns => reaction_elapsed_ns_per_second,
    framework_elapsed_ns => framework_elapsed_ns_per_second,
    physical_wait_elapsed_ns => physical_wait_elapsed_ns_per_second,
    external_wait_elapsed_ns => external_wait_elapsed_ns_per_second,
    coordination_wait_elapsed_ns => coordination_wait_elapsed_ns_per_second,
    processed_tags => processed_tags_per_second,
    processed_reactions => processed_reactions_per_second,
    processed_events => processed_events_per_second,
    set_ports => set_ports_per_second,
    scheduled_actions => scheduled_actions_per_second,
    completed_logical_tags => completed_logical_tags_per_second,
}

/// One accepted scheduler observation with its already-derived rates.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct SchedulerSample {
    /// Scheduler-group sequence carried by this accepted record.
    pub sequence: u64,
    /// Producer monotonic timestamp at encoding, in nanoseconds.
    pub sender_monotonic_ns: u64,
    /// Producer monotonic timestamp of the scheduler observation, in nanoseconds.
    pub observation_monotonic_ns: u64,
    /// Receiver-local elapsed time at datagram receipt.
    pub received_at: Duration,
    /// Raw scheduler observation carried directly by the telemetry record.
    pub raw: ObservationSnapshot,
    /// Rates derived from this sample and its accepted predecessor.
    pub rates: SchedulerRates,
}

/// One accepted exporter-health record, independent of scheduler observations.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct ExporterHealthSample {
    /// Exporter-health group sequence carried by this accepted record.
    pub sequence: u64,
    /// Producer monotonic timestamp at encoding, in nanoseconds.
    pub sender_monotonic_ns: u64,
    /// Producer monotonic timestamp of the observation, in nanoseconds.
    pub observation_monotonic_ns: u64,
    /// Receiver-local elapsed time at datagram receipt.
    pub received_at: Duration,
    /// Cumulative exporter-health counters carried by the record.
    pub raw: ExporterHealth,
}

/// Scheduler-specific state retained for one telemetry source.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SchedulerSnapshot {
    /// Ordering, freshness, and discontinuity state for scheduler records.
    pub sequence: SequenceSnapshot,
    /// Most recent accepted scheduler sample.
    pub latest: SchedulerSample,
    /// Accepted samples in observation arrival order, oldest retained first.
    pub history: Vec<SchedulerSample>,
    /// Includes samples not retained when configured history capacity is zero.
    pub history_evictions: u64,
}

/// Exporter-health state retained independently for one telemetry source.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ExporterHealthSnapshot {
    /// Ordering and freshness state for exporter-health records.
    pub sequence: SequenceSnapshot,
    /// Most recent accepted exporter-health sample.
    pub latest: ExporterHealthSample,
}

/// All retained telemetry state for one complete source identity.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SourceSnapshot {
    /// Complete stable identity of this source.
    pub identity: SourceIdentitySnapshot,
    /// Scheduler state, if the source has accepted a scheduler record.
    pub scheduler: Option<SchedulerSnapshot>,
    /// Exporter-health state, if the source has accepted an exporter-health record.
    pub exporter_health: Option<ExporterHealthSnapshot>,
}

/// Deterministically ordered rendering input. Creating it cannot mutate the receiver.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct MonitorSnapshot {
    /// Aggregate receiver input and bounded-state counters.
    pub counters: ReceiverCounters,
    /// Sources in deterministic complete-identity order.
    pub sources: Vec<SourceSnapshot>,
}

#[derive(Default)]
struct SourceState {
    scheduler_sequence: SequenceSnapshot,
    exporter_sequence: SequenceSnapshot,
    scheduler: Option<SchedulerSample>,
    exporter_health: Option<ExporterHealthSample>,
    history: VecDeque<SchedulerSample>,
    history_evictions: u64,
}

impl SourceState {
    fn ingest(
        &mut self,
        record: TelemetryRecord<'_>,
        received_at: Duration,
        history_capacity: usize,
        counters: &mut ReceiverCounters,
    ) -> IngestOutcome {
        let sequence = match record.group {
            RecordGroup::Scheduler => &mut self.scheduler_sequence,
            RecordGroup::ExporterHealth => &mut self.exporter_sequence,
        };
        if !sequence.accept(record.group_sequence, received_at, counters) {
            return IngestOutcome::StaleOrReordered;
        }
        match record.value {
            TelemetryValue::Scheduler(raw) => {
                let (rates, discontinuity) = self.scheduler.as_ref().map_or(
                    (SchedulerRates::default(), false),
                    |previous| {
                        SchedulerRates::between(previous, record.observation_monotonic_ns, &raw)
                    },
                );
                if discontinuity {
                    sequence.discontinuities = sequence.discontinuities.saturating_add(1);
                    counters.discontinuities = counters.discontinuities.saturating_add(1);
                }
                let sample = SchedulerSample {
                    sequence: record.group_sequence,
                    sender_monotonic_ns: record.sender_monotonic_ns,
                    observation_monotonic_ns: record.observation_monotonic_ns,
                    received_at,
                    raw,
                    rates,
                };
                self.scheduler = Some(sample);
                if self.history.len() == history_capacity {
                    self.history.pop_front();
                    self.history_evictions = self.history_evictions.saturating_add(1);
                    counters.history_evictions = counters.history_evictions.saturating_add(1);
                }
                if history_capacity > 0 {
                    self.history.push_back(sample);
                }
            }
            TelemetryValue::ExporterHealth(raw) => {
                self.exporter_health = Some(ExporterHealthSample {
                    sequence: record.group_sequence,
                    sender_monotonic_ns: record.sender_monotonic_ns,
                    observation_monotonic_ns: record.observation_monotonic_ns,
                    received_at,
                    raw,
                });
            }
        }
        counters.accepted = counters.accepted.saturating_add(1);
        IngestOutcome::Accepted
    }
}

/// Bounded decoded-record state machine, with no transport or clock ownership.
pub struct Receiver {
    config: ReceiverConfig,
    sources: BTreeMap<SourceKey, SourceState>,
    metadata_bytes: usize,
    counters: ReceiverCounters,
}

impl Receiver {
    /// Creates empty bounded receiver state from explicit limits.
    pub fn new(config: ReceiverConfig) -> Self {
        Self {
            config,
            sources: BTreeMap::new(),
            metadata_bytes: 0,
            counters: ReceiverCounters::default(),
        }
    }

    /// Decode before registering a source; reject stale group input before
    /// changing samples, rates, history, or receipt freshness.
    /// Decodes and incorporates one datagram using temporary bounded scratch storage.
    pub fn ingest(&mut self, datagram: &[u8], received_at: Duration) -> IngestOutcome {
        let mut scratch = [0; MAX_DATAGRAM_BYTES];
        self.ingest_with_scratch(datagram, received_at, &mut scratch)
    }

    /// Winsock reports an oversized UDP datagram as a receive error, without
    /// returning bytes to decode or a source to register.
    #[cfg(windows)]
    pub(crate) fn record_oversized_datagram(&mut self) {
        self.counters.malformed.record(&CodecError::Oversize);
    }

    /// Transport path for reusing caller-owned codec scratch across datagrams.
    pub(crate) fn ingest_with_scratch(
        &mut self,
        datagram: &[u8],
        received_at: Duration,
        scratch: &mut [u8; MAX_DATAGRAM_BYTES],
    ) -> IngestOutcome {
        let record = match TelemetryRecord::decode(datagram, scratch) {
            Ok(record) => record,
            Err(error) => {
                self.counters.malformed.record(&error);
                return IngestOutcome::Malformed(error);
            }
        };
        // Scan borrowed identities within the bounded registry. This avoids
        // allocating any identity strings for an existing or rejected source.
        if let Some((_, source)) = self
            .sources
            .iter_mut()
            .find(|(key, _)| key.matches(&record))
        {
            return source.ingest(
                record,
                received_at,
                self.config.history_capacity,
                &mut self.counters,
            );
        }
        if self.sources.len() >= self.config.max_sources {
            self.counters.source_limit_rejected =
                self.counters.source_limit_rejected.saturating_add(1);
            return IngestOutcome::SourceLimitRejected;
        }
        // Decoding bounds these borrowed string lengths by MAX_DATAGRAM_BYTES.
        let metadata_bytes = record.process_id.len()
            + record.source.federate_id.map_or(0, str::len)
            + record.source.enclave_id.map_or(0, str::len);
        if metadata_bytes > self.config.max_metadata_bytes - self.metadata_bytes {
            self.counters.metadata_limit_rejected =
                self.counters.metadata_limit_rejected.saturating_add(1);
            return IngestOutcome::MetadataLimitRejected;
        }
        let key = SourceKey::copy_from(&record);
        self.metadata_bytes += metadata_bytes;
        self.sources.entry(key).or_default().ingest(
            record,
            received_at,
            self.config.history_capacity,
            &mut self.counters,
        )
    }

    /// Returns a deterministic snapshot without changing receiver state.
    pub fn snapshot(&self, now: Duration) -> MonitorSnapshot {
        MonitorSnapshot {
            counters: self.counters,
            sources: self
                .sources
                .iter()
                .map(|(key, source)| SourceSnapshot {
                    identity: key.0.clone(),
                    scheduler: source.scheduler.map(|latest| SchedulerSnapshot {
                        sequence: source.scheduler_sequence.at(now),
                        latest,
                        history: source.history.iter().copied().collect(),
                        history_evictions: source.history_evictions,
                    }),
                    exporter_health: source.exporter_health.map(|latest| ExporterHealthSnapshot {
                        sequence: source.exporter_sequence.at(now),
                        latest,
                    }),
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{Receiver, ReceiverConfig};
    use boomerang_telemetry::{
        RecordGroup, SourceIdentity, SourceRole, TelemetryRecord, TelemetryValue,
        MAX_DATAGRAM_BYTES,
    };
    use std::time::Duration;

    fn scheduler(sequence: u64, observation_ns: u64, count: u64) -> TelemetryRecord<'static> {
        let raw = serde_json::from_value(serde_json::json!({
            "lifecycle": "Running", "current_phase": "Framework",
            "current_phase_started_ns": 42,
            "reaction_elapsed_ns": count, "framework_elapsed_ns": count,
            "physical_wait_elapsed_ns": count, "external_wait_elapsed_ns": count,
            "coordination_wait_elapsed_ns": count, "processed_tags": count,
            "processed_reactions": count, "processed_events": count,
            "set_ports": count, "scheduled_actions": count,
            "event_queue_occupancy": 7, "event_queue_reserved_capacity": 16,
            "event_queue_enforced_limit": null, "event_queue_peak_occupancy": 9,
            "completed_logical_tags": count, "last_logical_progress_ns": 23
        }))
        .unwrap();
        TelemetryRecord {
            protocol_version: 1,
            group: RecordGroup::Scheduler,
            group_sequence: sequence,
            run_id: [1; 16],
            artifact_id: [2; 32],
            process_id: "pid",
            process_incarnation: 3,
            source: SourceIdentity {
                role: SourceRole::Federate,
                federate_id: Some("fed"),
                enclave_id: Some("enc"),
            },
            sender_monotonic_ns: 99,
            observation_monotonic_ns: observation_ns,
            value: TelemetryValue::Scheduler(raw),
        }
    }

    fn exporter(sequence: u64) -> TelemetryRecord<'static> {
        let mut record = scheduler(sequence, 500, 0);
        record.group = RecordGroup::ExporterHealth;
        record.value = TelemetryValue::ExporterHealth(boomerang_telemetry::ExporterHealth {
            publication_drops: 3,
            snapshot_misses: 4,
        });
        record
    }

    fn encode(record: TelemetryRecord<'_>) -> Vec<u8> {
        let mut bytes = [0; MAX_DATAGRAM_BYTES];
        let len = record.encode_into(&mut bytes).unwrap();
        bytes[..len].to_vec()
    }

    #[test]
    fn scheduler_rates_use_observation_time_and_preserve_raw_gauges() {
        let mut receiver = Receiver::new(ReceiverConfig::default());
        receiver.ingest(&encode(scheduler(0, 1_000, 10)), Duration::from_secs(90));
        receiver.ingest(&encode(scheduler(1, 2_000, 30)), Duration::from_secs(91));
        let snapshot = receiver.snapshot(Duration::from_secs(99));
        let scheduler = snapshot.sources[0].scheduler.as_ref().unwrap();
        let rates = serde_json::to_value(scheduler.latest.rates).unwrap();
        assert_eq!(rates.as_object().unwrap().len(), 11);
        for rate in rates.as_object().unwrap().values() {
            assert_eq!(rate, &serde_json::json!(20_000_000));
        }
        assert_eq!(scheduler.latest.raw.event_queue_occupancy, 7);
        assert_eq!(scheduler.latest.raw.event_queue_reserved_capacity, 16);
        assert_eq!(scheduler.latest.raw.event_queue_enforced_limit, None);
        assert_eq!(scheduler.latest.raw.event_queue_peak_occupancy, 9);
        assert_eq!(scheduler.latest.raw.current_phase_started_ns, 42);
        assert_eq!(scheduler.latest.raw.last_logical_progress_ns, Some(23));
        assert_eq!(scheduler.latest.sender_monotonic_ns, 99);
        assert_eq!(scheduler.sequence.age, Some(Duration::from_secs(8)));
        assert_eq!(snapshot.counters.accepted, 2);
    }

    #[test]
    fn duplicate_and_reordered_records_cannot_replace_values_or_freshness() {
        let mut receiver = Receiver::new(ReceiverConfig::default());
        receiver.ingest(&encode(scheduler(7, 1_000, 10)), Duration::ZERO);
        for sequence in [7, 6] {
            assert_eq!(
                receiver.ingest(
                    &encode(scheduler(sequence, 2_000, 99)),
                    Duration::from_secs(1)
                ),
                super::IngestOutcome::StaleOrReordered
            );
        }
        let snapshot = receiver.snapshot(Duration::from_secs(2));
        let scheduler = snapshot.sources[0].scheduler.as_ref().unwrap();
        assert_eq!(scheduler.history.len(), 1);
        assert_eq!(scheduler.latest.raw.processed_tags, 10);
        assert_eq!(scheduler.sequence.latest, Some(7));
        assert_eq!(scheduler.sequence.stale_or_reordered, 2);
        assert_eq!(scheduler.sequence.fresh_at, Some(Duration::ZERO));
        assert_eq!(scheduler.sequence.age, Some(Duration::from_secs(2)));
        assert_eq!(snapshot.counters.accepted, 1);
        assert_eq!(snapshot.counters.stale_or_reordered, 2);
    }

    #[test]
    fn exporter_health_has_independent_sequence_and_group_freshness() {
        let mut receiver = Receiver::new(ReceiverConfig::default());
        receiver.ingest(&encode(exporter(0)), Duration::from_secs(2));
        assert!(receiver.snapshot(Duration::from_secs(3)).sources[0]
            .scheduler
            .is_none());
        receiver.ingest(&encode(scheduler(100, 1_000, 10)), Duration::from_secs(4));
        receiver.ingest(&encode(exporter(1)), Duration::from_secs(8));
        let mut stale = exporter(0);
        stale.value = TelemetryValue::ExporterHealth(boomerang_telemetry::ExporterHealth {
            publication_drops: 90,
            snapshot_misses: 99,
        });
        receiver.ingest(&encode(stale), Duration::from_secs(9));
        let snapshot = receiver.snapshot(Duration::from_secs(10));
        let source = &snapshot.sources[0];
        let scheduler = source.scheduler.as_ref().unwrap();
        let exporter = source.exporter_health.as_ref().unwrap();
        assert_eq!(scheduler.history.len(), 1);
        assert_eq!(scheduler.sequence.age, Some(Duration::from_secs(6)));
        assert_eq!(scheduler.sequence.latest, Some(100));
        assert_eq!(exporter.sequence.latest, Some(1));
        assert_eq!(exporter.sequence.age, Some(Duration::from_secs(2)));
        assert_eq!(exporter.sequence.stale_or_reordered, 1);
        assert_eq!(exporter.latest.raw.publication_drops, 3);
        assert_eq!(exporter.latest.raw.snapshot_misses, 4);
    }

    #[test]
    fn every_identity_component_distinguishes_sources_and_is_copied() {
        let original = scheduler(0, 1_000, 10);
        let mut variants = vec![original; 11];
        variants[1].run_id = [9; 16];
        variants[2].artifact_id = [9; 32];
        variants[3].process_id = "other";
        variants[4].process_incarnation = 4;
        variants[5].source.role = SourceRole::Standalone;
        variants[6].source.federate_id = Some("other");
        variants[7].source.enclave_id = Some("other");
        variants[8].source.enclave_id = None;
        variants[9].source.enclave_id = Some("");
        variants[10].source.role = SourceRole::CentralRti;
        let mut receiver = Receiver::new(ReceiverConfig::default());
        for record in variants.iter().rev() {
            let mut bytes = encode(*record);
            receiver.ingest(&bytes, Duration::ZERO);
            bytes.fill(0);
        }
        let snapshot = receiver.snapshot(Duration::ZERO);
        assert_eq!(snapshot.sources.len(), 11);
        assert_eq!(snapshot.sources[0].identity.process_id, "other");
        let mut forward = Receiver::new(ReceiverConfig::default());
        for record in variants {
            forward.ingest(&encode(record), Duration::ZERO);
        }
        assert_eq!(snapshot, forward.snapshot(Duration::ZERO));
        for source in snapshot.sources {
            let scheduler = source.scheduler.unwrap();
            assert_eq!(scheduler.latest.rates.processed_tags_per_second, None);
            assert_eq!(scheduler.sequence.skipped, 0);
        }
    }

    #[test]
    fn count_and_aggregate_metadata_limits_reject_only_new_sources() {
        for (config, count_rejected, metadata_rejected) in [
            (
                ReceiverConfig {
                    max_sources: 1,
                    history_capacity: 2,
                    max_metadata_bytes: 64,
                },
                1,
                0,
            ),
            (
                ReceiverConfig {
                    max_sources: 2,
                    history_capacity: 2,
                    max_metadata_bytes: 9,
                },
                0,
                1,
            ),
        ] {
            let mut receiver = Receiver::new(config);
            receiver.ingest(&encode(scheduler(0, 1_000, 10)), Duration::ZERO);
            let mut other = scheduler(0, 1_000, 10);
            other.process_incarnation += 1;
            receiver.ingest(&encode(other), Duration::ZERO);
            receiver.ingest(&encode(scheduler(1, 2_000, 20)), Duration::ZERO);
            let snapshot = receiver.snapshot(Duration::ZERO);
            assert_eq!(snapshot.sources.len(), 1);
            assert_eq!(snapshot.counters.accepted, 2);
            assert_eq!(snapshot.counters.source_limit_rejected, count_rejected);
            assert_eq!(snapshot.counters.metadata_limit_rejected, metadata_rejected);
        }
    }

    #[test]
    fn zero_limits_reject_registration_without_panicking() {
        for config in [
            ReceiverConfig {
                max_sources: 0,
                ..ReceiverConfig::default()
            },
            ReceiverConfig {
                max_metadata_bytes: 0,
                ..ReceiverConfig::default()
            },
        ] {
            let mut receiver = Receiver::new(config);
            receiver.ingest(&encode(scheduler(0, 1_000, 10)), Duration::ZERO);
            let snapshot = receiver.snapshot(Duration::ZERO);
            assert!(snapshot.sources.is_empty());
            assert_eq!(snapshot.counters.rejected(), 1);
        }
    }

    #[test]
    fn gapped_sequences_count_skips_and_evict_oldest_history() {
        let mut receiver = Receiver::new(ReceiverConfig {
            history_capacity: 2,
            ..ReceiverConfig::default()
        });
        for (sequence, time, count) in [(5, 1_000, 10), (8, 2_000, 30), (9, 3_000, 50)] {
            receiver.ingest(&encode(scheduler(sequence, time, count)), Duration::ZERO);
        }
        let snapshot = receiver.snapshot(Duration::ZERO);
        let scheduler = snapshot.sources[0].scheduler.as_ref().unwrap();
        assert_eq!(scheduler.sequence.skipped, 2);
        assert_eq!(scheduler.history.len(), 2);
        assert_eq!(scheduler.history[0].sequence, 8);
        assert_eq!(scheduler.history[1].sequence, 9);
        assert_eq!(scheduler.history_evictions, 1);
        assert_eq!(
            scheduler.latest.rates.processed_tags_per_second,
            Some(20_000_000)
        );
        assert_eq!(snapshot.counters.history_evictions, 1);
        assert_eq!(snapshot.counters.skipped_sequences, 2);
    }

    #[test]
    fn zero_history_capacity_still_updates_latest_and_rates() {
        let mut receiver = Receiver::new(ReceiverConfig {
            history_capacity: 0,
            ..ReceiverConfig::default()
        });
        receiver.ingest(&encode(scheduler(0, 1_000, 10)), Duration::ZERO);
        receiver.ingest(&encode(scheduler(1, 2_000, 30)), Duration::ZERO);
        let snapshot = receiver.snapshot(Duration::ZERO);
        let scheduler = snapshot.sources[0].scheduler.as_ref().unwrap();
        assert!(scheduler.history.is_empty());
        assert_eq!(
            scheduler.latest.rates.processed_tags_per_second,
            Some(20_000_000)
        );
        assert_eq!(scheduler.history_evictions, 2);
    }

    #[test]
    fn counter_resets_saturation_and_nonmonotonic_time_invalidate_sample_rates() {
        for (prior_time, time, prior_count, count) in [
            (2_000, 1_000, 20, 10),
            (1_000, 1_000, 10, 20),
            (1_000, 2_000, 20, 10),
            (1_000, 2_000, 10, u64::MAX),
            (1_000, 2_000, u64::MAX, u64::MAX),
        ] {
            let mut receiver = Receiver::new(ReceiverConfig::default());
            receiver.ingest(
                &encode(scheduler(0, prior_time, prior_count)),
                Duration::ZERO,
            );
            receiver.ingest(&encode(scheduler(1, time, count)), Duration::ZERO);
            let snapshot = receiver.snapshot(Duration::ZERO);
            let scheduler = snapshot.sources[0].scheduler.as_ref().unwrap();
            for rate in serde_json::to_value(scheduler.latest.rates)
                .unwrap()
                .as_object()
                .unwrap()
                .values()
            {
                assert!(rate.is_null());
            }
            assert_eq!(scheduler.sequence.discontinuities, 1);
            assert_eq!(snapshot.counters.discontinuities, 1);
        }
    }

    #[test]
    fn single_counter_discontinuity_invalidates_all_rates_and_next_sample_recovers() {
        let mut receiver = Receiver::new(ReceiverConfig::default());
        receiver.ingest(&encode(scheduler(0, 1_000, 10)), Duration::ZERO);
        let mut reset = scheduler(1, 2_000, 20);
        if let TelemetryValue::Scheduler(raw) = &mut reset.value {
            raw.completed_logical_tags = 9;
        }
        receiver.ingest(&encode(reset), Duration::ZERO);
        assert_eq!(
            receiver.snapshot(Duration::ZERO).sources[0]
                .scheduler
                .as_ref()
                .unwrap()
                .latest
                .rates
                .processed_tags_per_second,
            None
        );
        receiver.ingest(&encode(scheduler(2, 3_000, 30)), Duration::ZERO);
        let snapshot = receiver.snapshot(Duration::ZERO);
        let scheduler = snapshot.sources[0].scheduler.as_ref().unwrap();
        assert_eq!(
            scheduler.latest.rates.processed_tags_per_second,
            Some(10_000_000)
        );
        assert_eq!(
            scheduler.latest.rates.completed_logical_tags_per_second,
            Some(21_000_000)
        );
        assert_eq!(scheduler.sequence.discontinuities, 1);
    }

    #[test]
    fn largest_sequence_does_not_wrap_and_unchanged_counters_have_zero_rates() {
        let mut receiver = Receiver::new(ReceiverConfig::default());
        receiver.ingest(&encode(scheduler(0, 1_000, 10)), Duration::ZERO);
        receiver.ingest(&encode(scheduler(u64::MAX, 2_000, 10)), Duration::ZERO);
        receiver.ingest(&encode(scheduler(0, 3_000, 30)), Duration::ZERO);
        let snapshot = receiver.snapshot(Duration::ZERO);
        let scheduler = snapshot.sources[0].scheduler.as_ref().unwrap();
        assert_eq!(scheduler.sequence.skipped, u64::MAX - 1);
        assert_eq!(scheduler.sequence.stale_or_reordered, 1);
        assert_eq!(scheduler.latest.rates.processed_tags_per_second, Some(0));
    }

    #[test]
    fn malformed_categories_never_register_sources() {
        let good = encode(scheduler(0, 1_000, 10));
        let mut version = good.clone();
        version[0] = 2;
        let mut group = good.clone();
        group[1] = 1;
        let mut trailing = good.clone();
        trailing.push(0);
        let mut noncanonical = good.clone();
        noncanonical.splice(2..3, [0x80, 0]);
        let inputs = [
            vec![],
            vec![0; MAX_DATAGRAM_BYTES + 1],
            version,
            group,
            trailing,
            noncanonical,
        ];
        let mut receiver = Receiver::new(ReceiverConfig::default());
        for input in inputs {
            receiver.ingest(&input, Duration::ZERO);
        }
        let snapshot = receiver.snapshot(Duration::ZERO);
        assert!(snapshot.sources.is_empty());
        assert_eq!(snapshot.counters.accepted, 0);
        assert_eq!(snapshot.counters.rejected(), 6);
        assert_eq!(snapshot.counters.malformed.codec, 1);
        assert_eq!(snapshot.counters.malformed.oversize, 1);
        assert_eq!(snapshot.counters.malformed.unsupported_version, 1);
        assert_eq!(snapshot.counters.malformed.group_mismatch, 1);
        assert_eq!(snapshot.counters.malformed.trailing_data, 1);
        assert_eq!(snapshot.counters.malformed.noncanonical, 1);
        receiver.ingest(&good, Duration::ZERO);
        assert_eq!(receiver.snapshot(Duration::ZERO).sources.len(), 1);
    }

    #[test]
    fn snapshots_are_serializable_and_age_does_not_change_receiver_state() {
        let mut receiver = Receiver::new(ReceiverConfig::default());
        receiver.ingest(&encode(scheduler(0, 1_000, 10)), Duration::from_secs(5));
        let before = receiver.snapshot(Duration::from_secs(4));
        assert_eq!(
            before.sources[0].scheduler.as_ref().unwrap().sequence.age,
            None
        );
        let later = receiver.snapshot(Duration::from_secs(8));
        assert_eq!(
            later.sources[0].scheduler.as_ref().unwrap().sequence.age,
            Some(Duration::from_secs(3))
        );
        let json = serde_json::to_value(receiver.snapshot(Duration::from_secs(5))).unwrap();
        assert_eq!(
            json["sources"][0]["scheduler"]["latest"]["raw"]["lifecycle"],
            "Running"
        );
        assert_eq!(
            json["sources"][0]["scheduler"]["latest"]["raw"]["current_phase"],
            "Framework"
        );
        assert_eq!(json["counters"]["accepted"], 1);
    }

    #[test]
    fn every_cumulative_counter_uses_its_own_delta_and_detects_discontinuities() {
        let fields = [
            (
                "reaction_elapsed_ns",
                "reaction_elapsed_ns_per_second",
                11,
                500_000,
            ),
            (
                "framework_elapsed_ns",
                "framework_elapsed_ns_per_second",
                12,
                1_000_000,
            ),
            (
                "physical_wait_elapsed_ns",
                "physical_wait_elapsed_ns_per_second",
                13,
                1_500_000,
            ),
            (
                "external_wait_elapsed_ns",
                "external_wait_elapsed_ns_per_second",
                14,
                2_000_000,
            ),
            (
                "coordination_wait_elapsed_ns",
                "coordination_wait_elapsed_ns_per_second",
                15,
                2_500_000,
            ),
            ("processed_tags", "processed_tags_per_second", 16, 3_000_000),
            (
                "processed_reactions",
                "processed_reactions_per_second",
                17,
                3_500_000,
            ),
            (
                "processed_events",
                "processed_events_per_second",
                18,
                4_000_000,
            ),
            ("set_ports", "set_ports_per_second", 19, 4_500_000),
            (
                "scheduled_actions",
                "scheduled_actions_per_second",
                20,
                5_000_000,
            ),
            (
                "completed_logical_tags",
                "completed_logical_tags_per_second",
                21,
                5_500_000,
            ),
        ];
        let baseline = scheduler(0, 1_000, 10);
        for (counter_field, rate_field, count, expected_rate) in fields {
            for (counter, expected) in [(count, Some(expected_rate)), (9, None), (u64::MAX, None)] {
                let mut next = scheduler(1, 3_000, 10);
                if let TelemetryValue::Scheduler(raw) = &mut next.value {
                    let mut json = serde_json::to_value(*raw).unwrap();
                    json[counter_field] = serde_json::json!(counter);
                    *raw = serde_json::from_value(json).unwrap();
                }
                let mut receiver = Receiver::new(ReceiverConfig::default());
                receiver.ingest(&encode(baseline), Duration::ZERO);
                receiver.ingest(&encode(next), Duration::ZERO);
                let snapshot = receiver.snapshot(Duration::ZERO);
                let scheduler = snapshot.sources[0].scheduler.as_ref().unwrap();
                let rates = serde_json::to_value(scheduler.latest.rates).unwrap();
                assert_eq!(
                    rates[rate_field],
                    serde_json::json!(expected),
                    "{counter_field}: {counter}"
                );
                assert_eq!(
                    scheduler.sequence.discontinuities,
                    u64::from(expected.is_none())
                );
            }
        }
    }

    #[test]
    fn rate_arithmetic_widens_before_multiplication_and_never_wraps() {
        for (time, count, expected) in [
            (2, 1, Some(333_333_333)),
            (2_999_999_999, u64::MAX - 1, Some(6_148_914_691_236_517_204)),
            (0, u64::MAX - 1, None),
        ] {
            let mut receiver = Receiver::new(ReceiverConfig::default());
            receiver.ingest(&encode(scheduler(0, 0, 0)), Duration::ZERO);
            receiver.ingest(&encode(scheduler(1, time + 1, count)), Duration::ZERO);
            let snapshot = receiver.snapshot(Duration::ZERO);
            let scheduler = snapshot.sources[0].scheduler.as_ref().unwrap();
            assert_eq!(scheduler.latest.rates.processed_tags_per_second, expected);
            assert_eq!(scheduler.sequence.discontinuities, 0);
        }
    }

    #[test]
    fn metadata_budget_measures_utf8_bytes_and_rejection_leaves_budget_available() {
        let mut receiver = Receiver::new(ReceiverConfig {
            max_metadata_bytes: 2,
            ..ReceiverConfig::default()
        });
        let mut record = scheduler(0, 1_000, 10);
        record.source.federate_id = None;
        record.source.enclave_id = None;
        record.process_id = "éé";
        assert_eq!(
            receiver.ingest(&encode(record), Duration::ZERO),
            super::IngestOutcome::MetadataLimitRejected
        );
        record.process_id = "é";
        assert_eq!(
            receiver.ingest(&encode(record), Duration::ZERO),
            super::IngestOutcome::Accepted
        );
        let snapshot = receiver.snapshot(Duration::ZERO);
        assert_eq!(snapshot.sources.len(), 1);
        assert_eq!(snapshot.sources[0].identity.process_id, "é");
        assert_eq!(snapshot.counters.metadata_limit_rejected, 1);
    }
}
