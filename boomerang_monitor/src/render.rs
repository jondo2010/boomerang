use std::{
    fmt::{self, Display, Write},
    time::Duration,
};

use crate::{MonitorSnapshot, SequenceSnapshot};

/// Diagnostic, human-readable output. This is intentionally not a stable or
/// structured serialization format; use [`render_json`] for structured output.
impl Display for MonitorSnapshot {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_diagnostic(output, self)
    }
}

fn write_diagnostic(output: &mut impl Write, snapshot: &MonitorSnapshot) -> fmt::Result {
    let counters = &snapshot.counters;
    writeln!(
        output,
        "receiver accepted={} records malformed={} records source_limit_rejected={} records metadata_limit_rejected={} records stale_or_reordered={} records skipped_sequences={} sequences discontinuities={} intervals history_evictions={} samples",
        counters.accepted,
        counters.malformed.total(),
        counters.source_limit_rejected,
        counters.metadata_limit_rejected,
        counters.stale_or_reordered,
        counters.skipped_sequences,
        counters.discontinuities,
        counters.history_evictions,
    )
    ?;
    writeln!(
        output,
        "malformed oversize={} codec={} trailing_data={} noncanonical={} unsupported_version={} group_mismatch={} buffer_too_small={} records",
        counters.malformed.oversize,
        counters.malformed.codec,
        counters.malformed.trailing_data,
        counters.malformed.noncanonical,
        counters.malformed.unsupported_version,
        counters.malformed.group_mismatch,
        counters.malformed.buffer_too_small,
    )
    ?;

    for source in &snapshot.sources {
        let identity = &source.identity;
        output.write_str("source run_id=")?;
        write_hex(output, &identity.run_id)?;
        output.write_str(" artifact_id=")?;
        write_hex(output, &identity.artifact_id)?;
        writeln!(
            output,
            " process_id={:?} process_incarnation={} role={:?} federate_id={:?} enclave_id={:?}",
            identity.process_id,
            identity.process_incarnation,
            identity.role,
            identity.federate_id,
            identity.enclave_id,
        )?;

        if let Some(scheduler) = &source.scheduler {
            write_sequence(output, "scheduler", &scheduler.sequence)?;
            let sample = &scheduler.latest;
            let raw = &sample.raw;
            let rates = &sample.rates;
            writeln!(
                output,
                "  history_retained={} samples history_evictions={} samples",
                scheduler.history.len(),
                scheduler.history_evictions
            )?;
            write_sample_times(
                output,
                sample.sender_monotonic_ns,
                sample.observation_monotonic_ns,
                sample.received_at,
            )?;
            writeln!(
                output,
                "  lifecycle={:?} current_phase={:?}",
                raw.lifecycle, raw.current_phase
            )?;
            write_line(
                output,
                "current_phase_started_ns",
                raw.current_phase_started_ns,
                "ns",
            )?;
            write_line(
                output,
                "event_queue_occupancy",
                raw.event_queue_occupancy,
                "events",
            )?;
            write_line(
                output,
                "event_queue_reserved_capacity",
                raw.event_queue_reserved_capacity,
                "events",
            )?;
            write_optional_line(
                output,
                "event_queue_enforced_limit",
                raw.event_queue_enforced_limit,
                "events",
            )?;
            write_line(
                output,
                "event_queue_peak_occupancy",
                raw.event_queue_peak_occupancy,
                "events",
            )?;
            write_optional_line(
                output,
                "last_logical_progress_ns",
                raw.last_logical_progress_ns,
                "ns",
            )?;

            macro_rules! counter_and_rate {
                ($raw_field:ident, $rate_field:ident, $raw_unit:literal, $rate_unit:literal) => {{
                    write_line(output, stringify!($raw_field), raw.$raw_field, $raw_unit)?;
                    write_optional_line(
                        output,
                        stringify!($rate_field),
                        rates.$rate_field,
                        $rate_unit,
                    )?;
                }};
            }
            counter_and_rate!(
                reaction_elapsed_ns,
                reaction_elapsed_ns_per_second,
                "ns",
                "ns/s"
            );
            counter_and_rate!(
                framework_elapsed_ns,
                framework_elapsed_ns_per_second,
                "ns",
                "ns/s"
            );
            counter_and_rate!(
                physical_wait_elapsed_ns,
                physical_wait_elapsed_ns_per_second,
                "ns",
                "ns/s"
            );
            counter_and_rate!(
                external_wait_elapsed_ns,
                external_wait_elapsed_ns_per_second,
                "ns",
                "ns/s"
            );
            counter_and_rate!(
                coordination_wait_elapsed_ns,
                coordination_wait_elapsed_ns_per_second,
                "ns",
                "ns/s"
            );
            counter_and_rate!(processed_tags, processed_tags_per_second, "tags", "tags/s");
            counter_and_rate!(
                processed_reactions,
                processed_reactions_per_second,
                "reactions",
                "reactions/s"
            );
            counter_and_rate!(
                processed_events,
                processed_events_per_second,
                "events",
                "events/s"
            );
            counter_and_rate!(set_ports, set_ports_per_second, "ports", "ports/s");
            counter_and_rate!(
                scheduled_actions,
                scheduled_actions_per_second,
                "actions",
                "actions/s"
            );
            counter_and_rate!(
                completed_logical_tags,
                completed_logical_tags_per_second,
                "tags",
                "tags/s"
            );
        }

        if let Some(exporter) = &source.exporter_health {
            write_sequence(output, "exporter_health", &exporter.sequence)?;
            write_sample_times(
                output,
                exporter.latest.sender_monotonic_ns,
                exporter.latest.observation_monotonic_ns,
                exporter.latest.received_at,
            )?;
            write_line(
                output,
                "publication_drops",
                exporter.latest.raw.publication_drops,
                "records",
            )?;
            write_line(
                output,
                "snapshot_misses",
                exporter.latest.raw.snapshot_misses,
                "snapshots",
            )?;
        }
    }
    Ok(())
}

/// Serialize the exact snapshot model as pretty JSON.
/// This function does not update receiver state or derive rates.
pub fn render_json(snapshot: &MonitorSnapshot) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(snapshot)
}

fn write_hex(output: &mut impl Write, bytes: &[u8]) -> fmt::Result {
    for byte in bytes {
        write!(output, "{byte:02x}")?;
    }
    Ok(())
}

fn available<T: Display>(value: Option<T>) -> String {
    value.map_or_else(|| "unavailable".to_owned(), |value| value.to_string())
}

fn age(value: Option<Duration>) -> String {
    value.map_or_else(|| "unavailable".to_owned(), |value| format!("{value:?}"))
}

fn write_sequence(
    output: &mut impl Write,
    group: &str,
    sequence: &SequenceSnapshot,
) -> fmt::Result {
    writeln!(
        output,
        "{group} sequence={} age={} fresh_at={} skipped={} stale_or_reordered={} discontinuities={}",
        available(sequence.latest),
        age(sequence.age),
        age(sequence.fresh_at),
        sequence.skipped,
        sequence.stale_or_reordered,
        sequence.discontinuities,
    )
    ?;
    Ok(())
}

fn write_sample_times(
    output: &mut impl Write,
    sender_ns: u64,
    observation_ns: u64,
    received_at: Duration,
) -> fmt::Result {
    write_line(output, "sender_monotonic_ns", sender_ns, "ns")?;
    write_line(output, "observation_monotonic_ns", observation_ns, "ns")?;
    writeln!(output, "  received_at={received_at:?}")
}

fn write_line<T: Display>(
    output: &mut impl Write,
    name: &str,
    value: T,
    unit: &str,
) -> fmt::Result {
    writeln!(output, "  {name}={value} {unit}")
}

fn write_optional_line<T: Display>(
    output: &mut impl Write,
    name: &str,
    value: Option<T>,
    unit: &str,
) -> fmt::Result {
    write_line(output, name, available(value), unit)
}

#[cfg(test)]
mod tests {
    use super::render_json;
    use crate::{
        ExporterHealthSample, ExporterHealthSnapshot, MonitorSnapshot, ReceiverCounters,
        SequenceSnapshot, SourceIdentitySnapshot, SourceSnapshot,
    };
    use boomerang_telemetry::{ExporterHealth, SourceRole};
    use std::time::Duration;

    fn snapshot() -> MonitorSnapshot {
        MonitorSnapshot {
            counters: ReceiverCounters {
                accepted: 1,
                ..ReceiverCounters::default()
            },
            sources: vec![SourceSnapshot {
                identity: SourceIdentitySnapshot {
                    run_id: [1; 16],
                    artifact_id: [2; 32],
                    process_id: "pid".into(),
                    process_incarnation: 3,
                    role: SourceRole::Federate,
                    federate_id: Some("fed".into()),
                    enclave_id: Some("enc".into()),
                },
                scheduler: None,
                exporter_health: Some(ExporterHealthSnapshot {
                    sequence: SequenceSnapshot {
                        latest: Some(7),
                        fresh_at: Some(Duration::from_secs(4)),
                        age: Some(Duration::from_secs(1)),
                        ..SequenceSnapshot::default()
                    },
                    latest: ExporterHealthSample {
                        sequence: 7,
                        sender_monotonic_ns: 60,
                        observation_monotonic_ns: 55,
                        received_at: Duration::from_secs(4),
                        raw: ExporterHealth {
                            publication_drops: 3,
                            snapshot_misses: 4,
                        },
                    },
                }),
            }],
        }
    }

    #[test]
    fn json_serializes_the_exact_snapshot_without_mutation() {
        let snapshot = snapshot();
        let before = snapshot.clone();
        let json: serde_json::Value =
            serde_json::from_str(&render_json(&snapshot).unwrap()).unwrap();
        assert_eq!(json, serde_json::to_value(&before).unwrap());
        assert_eq!(json["counters"]["accepted"], 1);
        assert_eq!(
            json["sources"][0]["exporter_health"]["latest"]["raw"]["publication_drops"],
            3
        );
        assert_eq!(snapshot, before);
    }
}
