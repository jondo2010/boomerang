use std::{fmt::Display, fmt::Write, time::Duration};

use crate::{MonitorSnapshot, SequenceSnapshot};

/// Render one already-computed snapshot as deterministic human-readable text.
/// This function does not update receiver state or derive rates.
pub fn render_text(snapshot: &MonitorSnapshot) -> String {
    let mut output = String::new();
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
    .expect("writing to String cannot fail");
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
    .expect("writing to String cannot fail");

    for source in &snapshot.sources {
        let identity = &source.identity;
        output.push_str("source run_id=");
        write_hex(&mut output, &identity.run_id);
        output.push_str(" artifact_id=");
        write_hex(&mut output, &identity.artifact_id);
        writeln!(
            output,
            " process_id={:?} process_incarnation={} role={:?} federate_id={:?} enclave_id={:?}",
            identity.process_id,
            identity.process_incarnation,
            identity.role,
            identity.federate_id,
            identity.enclave_id,
        )
        .expect("writing to String cannot fail");

        if let Some(scheduler) = &source.scheduler {
            write_sequence(&mut output, "scheduler", &scheduler.sequence);
            let sample = &scheduler.latest;
            let raw = &sample.raw;
            let rates = &sample.rates;
            writeln!(
                output,
                "  history_retained={} samples history_evictions={} samples",
                scheduler.history.len(),
                scheduler.history_evictions
            )
            .expect("writing to String cannot fail");
            write_sample_times(
                &mut output,
                sample.sender_monotonic_ns,
                sample.observation_monotonic_ns,
                sample.received_at,
            );
            writeln!(
                output,
                "  lifecycle={:?} current_phase={:?}",
                raw.lifecycle, raw.current_phase
            )
            .expect("writing to String cannot fail");
            write_line(
                &mut output,
                "current_phase_started_ns",
                raw.current_phase_started_ns,
                "ns",
            );
            write_line(
                &mut output,
                "event_queue_occupancy",
                raw.event_queue_occupancy,
                "events",
            );
            write_line(
                &mut output,
                "event_queue_reserved_capacity",
                raw.event_queue_reserved_capacity,
                "events",
            );
            write_optional_line(
                &mut output,
                "event_queue_enforced_limit",
                raw.event_queue_enforced_limit,
                "events",
            );
            write_line(
                &mut output,
                "event_queue_peak_occupancy",
                raw.event_queue_peak_occupancy,
                "events",
            );
            write_optional_line(
                &mut output,
                "last_logical_progress_ns",
                raw.last_logical_progress_ns,
                "ns",
            );

            macro_rules! counter_and_rate {
                ($raw_field:ident, $rate_field:ident, $raw_unit:literal, $rate_unit:literal) => {
                    write_line(
                        &mut output,
                        stringify!($raw_field),
                        raw.$raw_field,
                        $raw_unit,
                    );
                    write_optional_line(
                        &mut output,
                        stringify!($rate_field),
                        rates.$rate_field,
                        $rate_unit,
                    );
                };
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
            write_sequence(&mut output, "exporter_health", &exporter.sequence);
            write_sample_times(
                &mut output,
                exporter.latest.sender_monotonic_ns,
                exporter.latest.observation_monotonic_ns,
                exporter.latest.received_at,
            );
            write_line(
                &mut output,
                "publication_drops",
                exporter.latest.raw.publication_drops,
                "records",
            );
            write_line(
                &mut output,
                "snapshot_misses",
                exporter.latest.raw.snapshot_misses,
                "snapshots",
            );
        }
    }
    output
}

/// Serialize the exact snapshot model as pretty JSON.
/// This function does not update receiver state or derive rates.
pub fn render_json(snapshot: &MonitorSnapshot) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(snapshot)
}

fn write_hex(output: &mut String, bytes: &[u8]) {
    for byte in bytes {
        write!(output, "{byte:02x}").expect("writing to String cannot fail");
    }
}

fn available<T: Display>(value: Option<T>) -> String {
    value.map_or_else(|| "unavailable".to_owned(), |value| value.to_string())
}

fn age(value: Option<Duration>) -> String {
    value.map_or_else(|| "unavailable".to_owned(), |value| format!("{value:?}"))
}

fn write_sequence(output: &mut String, group: &str, sequence: &SequenceSnapshot) {
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
    .expect("writing to String cannot fail");
}

fn write_sample_times(
    output: &mut String,
    sender_ns: u64,
    observation_ns: u64,
    received_at: Duration,
) {
    write_line(output, "sender_monotonic_ns", sender_ns, "ns");
    write_line(output, "observation_monotonic_ns", observation_ns, "ns");
    writeln!(output, "  received_at={received_at:?}").expect("writing to String cannot fail");
}

fn write_line<T: Display>(output: &mut String, name: &str, value: T, unit: &str) {
    writeln!(output, "  {name}={value} {unit}").expect("writing to String cannot fail");
}

fn write_optional_line<T: Display>(output: &mut String, name: &str, value: Option<T>, unit: &str) {
    write_line(output, name, available(value), unit);
}

#[cfg(test)]
mod tests {
    use super::{render_json, render_text};
    use crate::{
        ExporterHealthSample, ExporterHealthSnapshot, MonitorSnapshot, ReceiverCounters,
        SchedulerRates, SchedulerSample, SchedulerSnapshot, SequenceSnapshot,
        SourceIdentitySnapshot, SourceSnapshot,
    };
    use boomerang_telemetry::{ExporterHealth, ObservationSnapshot, SourceRole};
    use std::time::Duration;

    fn snapshot() -> MonitorSnapshot {
        let raw: ObservationSnapshot = serde_json::from_value(serde_json::json!({
            "lifecycle": "Running", "current_phase": "Framework",
            "current_phase_started_ns": 42,
            "reaction_elapsed_ns": 100, "framework_elapsed_ns": 101,
            "physical_wait_elapsed_ns": 102, "external_wait_elapsed_ns": 103,
            "coordination_wait_elapsed_ns": 104, "processed_tags": 10,
            "processed_reactions": 11, "processed_events": 12,
            "set_ports": 13, "scheduled_actions": 14,
            "event_queue_occupancy": 7, "event_queue_reserved_capacity": 16,
            "event_queue_enforced_limit": null, "event_queue_peak_occupancy": 9,
            "completed_logical_tags": 15, "last_logical_progress_ns": 23
        }))
        .unwrap();
        let rates = SchedulerRates {
            processed_tags_per_second: Some(20),
            ..SchedulerRates::default()
        };
        MonitorSnapshot {
            counters: ReceiverCounters {
                accepted: 2,
                stale_or_reordered: 1,
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
                scheduler: Some(SchedulerSnapshot {
                    sequence: SequenceSnapshot {
                        latest: Some(4),
                        fresh_at: Some(Duration::from_secs(3)),
                        age: Some(Duration::from_secs(2)),
                        ..SequenceSnapshot::default()
                    },
                    latest: SchedulerSample {
                        sequence: 4,
                        sender_monotonic_ns: 50,
                        observation_monotonic_ns: 40,
                        received_at: Duration::from_secs(3),
                        raw,
                        rates,
                    },
                    history: vec![],
                    history_evictions: 0,
                }),
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
    fn json_serializes_the_exact_snapshot_and_renderers_leave_it_unchanged() {
        let snapshot = snapshot();
        let before = snapshot.clone();
        let json: serde_json::Value =
            serde_json::from_str(&render_json(&snapshot).unwrap()).unwrap();
        assert_eq!(json, serde_json::to_value(&before).unwrap());
        assert_eq!(snapshot, before);
        assert_eq!(render_text(&snapshot), render_text(&snapshot));
        assert_eq!(snapshot, before);
    }

    #[test]
    fn text_reports_identity_freshness_gauges_and_stored_rate_units() {
        let rendered = render_text(&snapshot());
        let lines: Vec<_> = rendered.lines().collect();
        assert!(lines.iter().any(|line| line.contains("process_id=\"pid\"")));
        assert!(lines
            .iter()
            .any(|line| line.contains("scheduler sequence=4 age=2s")));
        assert!(lines
            .iter()
            .any(|line| line.contains("exporter_health sequence=7 age=1s")));
        assert!(lines.contains(&"  event_queue_occupancy=7 events"));
        assert!(lines.contains(&"  event_queue_enforced_limit=unavailable events"));
        assert!(lines.contains(&"  processed_tags=10 tags"));
        assert!(lines.contains(&"  processed_tags_per_second=20 tags/s"));
        assert!(lines.contains(&"  processed_reactions_per_second=unavailable reactions/s"));
        assert!(lines.contains(&"  publication_drops=3 records"));
    }
}
