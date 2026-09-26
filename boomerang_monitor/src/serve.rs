use std::{
    io,
    net::{SocketAddr, UdpSocket},
    num::NonZeroUsize,
    time::{Duration, Instant},
};

use boomerang_telemetry::MAX_DATAGRAM_BYTES;

use crate::{IngestOutcome, MonitorSnapshot, Receiver, ReceiverConfig};

/// UDP listener and finite-run controls for the hosted monitor.
#[derive(Clone, Copy, Debug)]
pub struct MonitorOptions {
    /// Local UDP address on which to receive telemetry.
    pub listen: SocketAddr,
    /// Caller-selected output mode; serving itself returns a snapshot.
    pub json: bool,
    /// Stop after this many accepted records; `None` listens indefinitely.
    pub max_records: Option<NonZeroUsize>,
    /// Maximum wait for the next datagram before reporting an idle failure.
    pub idle_timeout: Option<Duration>,
    /// Bounds for receiver-owned source and history state.
    pub receiver: ReceiverConfig,
}

/// Errors while binding, receiving, or presenting a monitor snapshot.
#[derive(Debug, thiserror::Error)]
pub enum MonitorError {
    /// Binding the requested local UDP address failed.
    #[error("cannot bind telemetry monitor UDP socket: {0}")]
    Bind(#[source] io::Error),
    /// Receiving a UDP datagram failed for a reason other than configured idleness.
    #[error("cannot receive telemetry monitor UDP datagram: {0}")]
    Receive(#[source] io::Error),
    /// The configured idle interval elapsed before the accepted-record target.
    #[error(
        "telemetry monitor idle timeout after {accepted} accepted and {rejected} rejected records"
    )]
    IdleTimeout {
        /// Records accepted before the idle deadline.
        accepted: u64,
        /// Records rejected before the idle deadline.
        rejected: u64,
    },
    /// Serializing the completed snapshot as JSON failed.
    #[error("cannot render telemetry monitor JSON: {0}")]
    Json(#[from] serde_json::Error),
}

/// Receive bounded UDP datagrams and return a snapshot once the accepted-record
/// target is reached. An idle timeout reports the counts accumulated so far.
pub fn serve(options: &MonitorOptions) -> Result<MonitorSnapshot, MonitorError> {
    let socket = UdpSocket::bind(options.listen).map_err(MonitorError::Bind)?;
    socket
        .set_read_timeout(options.idle_timeout)
        .map_err(MonitorError::Receive)?;
    let origin = Instant::now();
    let mut receiver = Receiver::new(options.receiver);
    // The extra byte distinguishes an oversized datagram from a valid full-size
    // record on platforms that return truncated UDP payloads.
    let mut buffer = [0; MAX_DATAGRAM_BYTES + 1];
    let mut scratch = [0; MAX_DATAGRAM_BYTES];
    let mut accepted = 0_usize;

    loop {
        match socket.recv_from(&mut buffer) {
            Ok((length, _sender)) => {
                let received_at = origin.elapsed();
                if matches!(
                    receiver.ingest_with_scratch(&buffer[..length], received_at, &mut scratch),
                    IngestOutcome::Accepted
                ) {
                    accepted = accepted.saturating_add(1);
                    if options
                        .max_records
                        .is_some_and(|limit| accepted >= limit.get())
                    {
                        return Ok(receiver.snapshot(received_at));
                    }
                }
            }
            #[cfg(windows)]
            Err(error) if error.raw_os_error() == Some(10040) => {
                // WSAEMSGSIZE consumes the datagram but returns no usable bytes.
                receiver.record_oversized_datagram();
            }
            Err(error)
                if options.idle_timeout.is_some()
                    && matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
            {
                let counters = receiver.snapshot(origin.elapsed()).counters;
                return Err(MonitorError::IdleTimeout {
                    accepted: counters.accepted,
                    rejected: counters.rejected(),
                });
            }
            Err(error) => return Err(MonitorError::Receive(error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{serve, MonitorError, MonitorOptions};
    use crate::ReceiverConfig;
    use boomerang_telemetry::{
        CodecError, ExporterHealth, RecordGroup, SourceIdentity, SourceRole, TelemetryRecord,
        TelemetryValue, MAX_DATAGRAM_BYTES,
    };
    use std::{net::UdpSocket, num::NonZeroUsize, time::Duration};

    fn local_options(max_records: usize, idle_timeout: Duration) -> MonitorOptions {
        let probe = UdpSocket::bind("127.0.0.1:0").unwrap();
        let listen = probe.local_addr().unwrap();
        drop(probe);
        MonitorOptions {
            listen,
            json: false,
            max_records: Some(NonZeroUsize::new(max_records).unwrap()),
            idle_timeout: Some(idle_timeout),
            receiver: ReceiverConfig::default(),
        }
    }

    fn health_record_for(process_id: &str) -> Result<Vec<u8>, CodecError> {
        let record = TelemetryRecord {
            protocol_version: 1,
            group: RecordGroup::ExporterHealth,
            group_sequence: 0,
            run_id: [1; 16],
            artifact_id: [2; 32],
            process_id,
            process_incarnation: 3,
            source: SourceIdentity {
                role: SourceRole::Federate,
                federate_id: Some("fed"),
                enclave_id: Some("enc"),
            },
            sender_monotonic_ns: 20,
            observation_monotonic_ns: 10,
            value: TelemetryValue::ExporterHealth(ExporterHealth {
                publication_drops: 3,
                snapshot_misses: 4,
            }),
        };
        let mut bytes = [0; MAX_DATAGRAM_BYTES];
        let len = record.encode_into(&mut bytes)?;
        Ok(bytes[..len].to_vec())
    }

    fn health_record() -> Vec<u8> {
        health_record_for("pid").unwrap()
    }

    fn full_health_record() -> Vec<u8> {
        (0..=MAX_DATAGRAM_BYTES)
            .find_map(|length| {
                let process_id = "p".repeat(length);
                let encoded = health_record_for(&process_id).ok()?;
                (encoded.len() == MAX_DATAGRAM_BYTES).then_some(encoded)
            })
            .expect("current codec can encode a canonical full-size health record")
    }

    #[test]
    fn serve_counts_only_accepted_datagrams_toward_the_finite_limit() {
        let options = local_options(1, Duration::from_secs(2));
        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        let endpoint = options.listen;
        let sender_thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            sender.send_to(&[0xff], endpoint).unwrap();
            sender.send_to(&health_record(), endpoint).unwrap();
        });
        let snapshot = serve(&options).unwrap();
        sender_thread.join().unwrap();
        assert_eq!(snapshot.counters.accepted, 1);
        assert_eq!(snapshot.counters.rejected(), 1);
        assert_eq!(snapshot.sources.len(), 1);
        assert_eq!(
            snapshot.sources[0]
                .exporter_health
                .as_ref()
                .unwrap()
                .latest
                .raw
                .publication_drops,
            3
        );
    }

    #[test]
    fn idle_timeout_reports_accepted_and_rejected_counts() {
        let options = local_options(1, Duration::from_millis(20));
        assert!(matches!(
            serve(&options).unwrap_err(),
            MonitorError::IdleTimeout {
                accepted: 0,
                rejected: 0
            }
        ));
    }

    #[test]
    fn idle_timeout_includes_rejected_datagrams_without_treating_them_as_progress() {
        let options = local_options(1, Duration::from_millis(100));
        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        let endpoint = options.listen;
        let sender_thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            sender.send_to(&[0xff], endpoint).unwrap();
        });
        let result = serve(&options);
        sender_thread.join().unwrap();
        assert!(matches!(
            result.unwrap_err(),
            MonitorError::IdleTimeout {
                accepted: 0,
                rejected: 1
            }
        ));
    }

    #[test]
    fn oversized_datagram_with_valid_full_size_prefix_does_not_reach_accepted_target() {
        let options = local_options(1, Duration::from_millis(200));
        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        let endpoint = options.listen;
        let mut datagram = full_health_record();
        datagram.push(0);
        assert_eq!(datagram.len(), MAX_DATAGRAM_BYTES + 1);
        let sender_thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            sender.send_to(&datagram, endpoint).unwrap();
        });
        let result = serve(&options);
        sender_thread.join().unwrap();
        assert!(matches!(
            result.unwrap_err(),
            MonitorError::IdleTimeout {
                accepted: 0,
                rejected: 1
            }
        ));
    }

    #[test]
    fn oversized_datagram_is_counted_in_the_oversize_category() {
        let options = local_options(1, Duration::from_secs(2));
        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        let endpoint = options.listen;
        let mut oversized = full_health_record();
        oversized.push(0);
        let sender_thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            sender.send_to(&oversized, endpoint).unwrap();
            sender.send_to(&health_record(), endpoint).unwrap();
        });
        let snapshot = serve(&options).unwrap();
        sender_thread.join().unwrap();
        assert_eq!(snapshot.counters.accepted, 1);
        assert_eq!(snapshot.counters.malformed.oversize, 1);
        assert_eq!(snapshot.counters.rejected(), 1);
    }

    #[test]
    fn datagram_larger_than_receive_buffer_is_rejected_without_stopping_serving() {
        let options = local_options(1, Duration::from_secs(2));
        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        let endpoint = options.listen;
        let mut oversized = full_health_record();
        oversized.extend_from_slice(&[0, 0]);
        assert!(oversized.len() > MAX_DATAGRAM_BYTES + 1);
        let sender_thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            sender.send_to(&oversized, endpoint).unwrap();
            sender.send_to(&health_record(), endpoint).unwrap();
        });
        let snapshot = serve(&options).unwrap();
        sender_thread.join().unwrap();
        assert_eq!(snapshot.counters.accepted, 1);
        assert_eq!(snapshot.counters.malformed.oversize, 1);
        assert_eq!(snapshot.counters.rejected(), 1);
        assert_eq!(snapshot.sources.len(), 1);
    }
}
