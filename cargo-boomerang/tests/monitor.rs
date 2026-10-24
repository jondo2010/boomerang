#[cfg(feature = "monitor")]
use std::net::UdpSocket;
use std::process::Command;

#[test]
#[cfg(feature = "monitor")]
fn monitor_rejects_invalid_options_before_binding() {
    let occupied = UdpSocket::bind("127.0.0.1:0").unwrap();
    let listen = occupied.local_addr().unwrap().to_string();
    for options in [
        vec!["--listen", "not-a-socket"],
        vec!["--listen", &listen, "--max-records", "0"],
        vec!["--listen", &listen, "--max-records", "invalid"],
        vec!["--listen", &listen, "--idle-timeout", "invalid"],
        vec!["--listen", &listen, "--idle-timeout", "0s"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_cargo-boomerang"))
            .args(["boomerang", "monitor"])
            .args(&options)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{options:?}: {output:?}");
        assert!(output.stdout.is_empty());
    }
}

#[test]
#[cfg(not(feature = "monitor"))]
fn monitor_requires_feature() {
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-boomerang"))
        .args(["boomerang", "monitor", "--help"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stdout.is_empty());
}

#[test]
#[cfg(feature = "monitor")]
fn monitor_receives_udp_telemetry_and_prints_json() {
    use boomerang_telemetry::{
        ExporterHealth, RecordGroup, SourceIdentity, SourceRole, TelemetryRecord, TelemetryValue,
        MAX_DATAGRAM_BYTES,
    };
    use std::{
        process::Stdio,
        time::{Duration, Instant},
    };

    /// Monitor process with cleanup after failed assertions.
    struct MonitorProcess {
        /// Child process that owns the telemetry listener.
        child: std::process::Child,
    }
    impl Drop for MonitorProcess {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    let probe = UdpSocket::bind("127.0.0.1:0").unwrap();
    let listen = probe.local_addr().unwrap();
    drop(probe);
    let mut monitor = MonitorProcess {
        child: Command::new(env!("CARGO_BIN_EXE_cargo-boomerang"))
            .args(["boomerang", "monitor", "--listen", &listen.to_string()])
            .args(["--json", "--max-records", "1", "--idle-timeout", "5s"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(
            monitor.child.try_wait().unwrap().is_none(),
            "monitor exited before binding"
        );
        match UdpSocket::bind(listen) {
            Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => break,
            Ok(probe) => drop(probe),
            Err(error) => panic!("failed to probe monitor listener: {error}"),
        }
        assert!(Instant::now() < deadline, "monitor did not bind");
        std::thread::sleep(Duration::from_millis(10));
    }
    let record = TelemetryRecord {
        protocol_version: 1,
        group: RecordGroup::ExporterHealth,
        group_sequence: 0,
        run_id: [1; 16],
        artifact_id: [2; 32],
        process_id: "native-vehicle-sensor",
        process_incarnation: 3,
        source: SourceIdentity {
            role: SourceRole::Federate,
            federate_id: Some("sensor"),
            enclave_id: Some("sensor"),
        },
        sender_monotonic_ns: 20,
        observation_monotonic_ns: 10,
        value: TelemetryValue::ExporterHealth(ExporterHealth {
            publication_drops: 0,
            snapshot_misses: 0,
        }),
    };
    let mut bytes = [0; MAX_DATAGRAM_BYTES];
    let length = record.encode_into(&mut bytes).unwrap();
    UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .send_to(&bytes[..length], listen)
        .unwrap();
    loop {
        if let Some(status) = monitor.child.try_wait().unwrap() {
            assert!(status.success(), "monitor failed: {status}");
            break;
        }
        assert!(Instant::now() < deadline, "monitor did not finish");
        std::thread::sleep(Duration::from_millis(10));
    }
    let snapshot: serde_json::Value =
        serde_json::from_reader(monitor.child.stdout.take().unwrap()).unwrap();
    assert_eq!(snapshot["counters"]["accepted"], 1);
    assert_eq!(
        snapshot["sources"][0]["identity"]["process_id"],
        "native-vehicle-sensor"
    );
    assert_eq!(snapshot["sources"][0]["identity"]["federate_id"], "sensor");
}
