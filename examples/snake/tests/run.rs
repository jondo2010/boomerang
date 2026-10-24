//! Execute the application through a Unix pseudo-terminal.

#[cfg(unix)]
#[test]
fn arrow_input_and_ctrl_c_restore_the_terminal() {
    let output = std::process::Command::new("python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/support/pty_run.py"
        ))
        .arg(env!("CARGO_BIN_EXE_snake"))
        .output()
        .expect("Unix PTY coverage requires Python 3");
    assert!(
        output.status.success(),
        "PTY regression failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    print!("{}", String::from_utf8_lossy(&output.stdout));
}

/// Catch missing exporter wiring, wrong source associations, and missing final snapshots.
#[cfg(all(unix, feature = "telemetry"))]
#[test]
fn telemetry_reports_both_schedulers_through_shutdown() {
    use boomerang_telemetry::{SourceRole, TelemetryRecord, TelemetryValue, MAX_DATAGRAM_BYTES};
    use std::{collections::BTreeMap, net::UdpSocket, time::Duration};

    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket
        .set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    let endpoint = socket.local_addr().unwrap();
    let child = std::thread::spawn(move || {
        std::process::Command::new("python3")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/support/pty_run.py"
            ))
            .arg(env!("CARGO_BIN_EXE_snake"))
            .env("BOOMERANG_TELEMETRY_ENDPOINT", endpoint.to_string())
            .env(
                "BOOMERANG_TELEMETRY_RUN_ID",
                "11111111111111111111111111111111",
            )
            .env("BOOMERANG_TELEMETRY_PERIOD_MS", "10")
            .output()
            .expect("Unix PTY coverage requires Python 3")
    });
    let mut samples = BTreeMap::<String, Vec<boomerang_runtime::ObservationSnapshot>>::new();
    let mut artifact = None;
    loop {
        let mut bytes = [0; MAX_DATAGRAM_BYTES];
        match socket.recv(&mut bytes) {
            Ok(length) => {
                let record =
                    TelemetryRecord::decode(&bytes[..length], &mut [0; MAX_DATAGRAM_BYTES])
                        .unwrap();
                assert_eq!(record.run_id, [0x11; 16]);
                assert_eq!(record.process_id, "host");
                assert_eq!(record.source.role, SourceRole::Federate);
                assert_eq!(record.source.federate_id, Some("host"));
                assert_eq!(
                    *artifact.get_or_insert(record.artifact_id),
                    record.artifact_id
                );
                if let TelemetryValue::Scheduler(snapshot) = record.value {
                    samples
                        .entry(record.source.enclave_id.unwrap().to_owned())
                        .or_default()
                        .push(snapshot);
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                if child.is_finished() {
                    break;
                }
            }
            Err(error) => panic!("telemetry receive failed: {error}"),
        }
    }
    let output = child.join().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        samples.keys().map(String::as_str).collect::<Vec<_>>(),
        ["keyboard", "snake"]
    );
    for (source, snapshots) in samples {
        assert!(
            snapshots
                .iter()
                .any(|sample| sample.lifecycle == boomerang_runtime::SchedulerLifecycle::Running),
            "{source}: no running snapshot"
        );
        let last = snapshots.last().unwrap();
        assert_eq!(
            last.lifecycle,
            boomerang_runtime::SchedulerLifecycle::Stopped,
            "{source}"
        );
        assert!(
            last.processed_reactions > 0,
            "{source}: no reaction observations"
        );
        assert!(last.processed_tags > 0, "{source}: no tag observations");
    }
}
