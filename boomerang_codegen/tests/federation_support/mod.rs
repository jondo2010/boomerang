use crate::cargo_support::{clean_environment, target_directory};

use std::{
    fs::{self, File},
    io::{BufRead, BufReader},
    net::{SocketAddr, TcpListener},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    time::{Duration, Instant},
};

const DEADLINE: Duration = Duration::from_secs(20);

/// Ordinary Cargo fixture paths and retained integration evidence.
struct Fixture {
    /// Standalone checked-in Cargo workspace.
    workspace: PathBuf,
    /// One ordinary target directory for the independent fixture builds.
    target: PathBuf,
    /// Retained build output, process output, and executable snapshots.
    evidence: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let target = target_directory("cargo-native-federation-slice4");
        let evidence = target.join("evidence");
        fs::create_dir_all(&evidence).unwrap();
        Self {
            workspace: root.join("tests/fixtures/federated"),
            target,
            evidence,
        }
    }

    fn cargo(&self, operation: &str, args: &[&str], label: &str, delay: Option<&str>) -> String {
        let mut command = Command::new(env!("CARGO"));
        command
            .current_dir(&self.workspace)
            .arg(operation)
            .args(["--locked", "--offline"])
            .args(args)
            .env("CARGO_TARGET_DIR", &self.target);
        clean_environment(&mut command);
        if let Some(delay) = delay {
            command.env("NATIVE_VEHICLE_DELAY_NS", delay);
        }
        let output = command.output().unwrap();
        let combined = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        fs::write(self.evidence.join(format!("{label}.log")), &combined).unwrap();
        assert!(output.status.success(), "{label}: {combined}");
        combined
    }

    fn build(
        &self,
        role: &str,
        label: &str,
        delay: Option<&str>,
        features: Option<&str>,
    ) -> PathBuf {
        let package = format!("native-vehicle-{role}");
        let mut args = vec!["-p", package.as_str()];
        if let Some(features) = features {
            args.extend(["--features", features]);
        }
        self.cargo("build", &args, label, delay);
        let artifact = self
            .evidence
            .join(format!("{label}{}", std::env::consts::EXE_SUFFIX));
        fs::copy(
            self.target
                .join("debug")
                .join(format!("{package}{}", std::env::consts::EXE_SUFFIX)),
            &artifact,
        )
        .unwrap();
        artifact
    }
}

fn identity(artifact: &Path) -> String {
    let output = Command::new(artifact).arg("--identity").output().unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap()
}

/// Child process with bounded completion and unconditional failure cleanup.
struct Process {
    /// Owned process handle, retained until exit or cleanup.
    child: Child,
    /// Retained stdout file for diagnostics and assertions.
    stdout: PathBuf,
    /// Retained stderr file for diagnostics and assertions.
    stderr: PathBuf,
}
impl Process {
    fn spawn(
        artifact: &Path,
        evidence: &Path,
        label: &str,
        environment: &[(&str, String)],
    ) -> Self {
        let stdout = evidence.join(format!("{label}.stdout"));
        let stderr = evidence.join(format!("{label}.stderr"));
        let mut command = Command::new(artifact);
        clean_environment(&mut command);
        let child = command
            .envs(environment.iter().map(|(key, value)| (*key, value)))
            .stdout(Stdio::from(File::create(&stdout).unwrap()))
            .stderr(Stdio::from(File::create(&stderr).unwrap()))
            .spawn()
            .unwrap();
        Self {
            child,
            stdout,
            stderr,
        }
    }
    fn output(&self) -> String {
        format!(
            "{}\n{}",
            fs::read_to_string(&self.stdout).unwrap(),
            fs::read_to_string(&self.stderr).unwrap()
        )
    }
    fn wait(&mut self, deadline: Instant) -> ExitStatus {
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "process exceeded deadline: {}",
                self.output()
            );
            std::thread::park_timeout(Duration::from_millis(5));
        }
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// Executable contracts exercised with the same bounded process harness.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Scenario {
    /// Matching members exchange a tagged payload and complete naturally.
    Matching,
    /// A changed semantic identity fails the real wire handshake.
    Mixed,
    /// A member exits inside a reaction after successful admission.
    RuntimeFailure,
}

fn run(fixture: &Fixture, artifacts: &[PathBuf; 3], scenario: Scenario) {
    let label = match scenario {
        Scenario::Matching => "matching",
        Scenario::Mixed => "mixed",
        Scenario::RuntimeFailure => "runtime-failure",
    };
    let ready = TcpListener::bind("127.0.0.1:0").unwrap();
    ready.set_nonblocking(true).unwrap();
    let deadline = Instant::now() + DEADLINE;
    let mut coordinator = Process::spawn(
        &artifacts[2],
        &fixture.evidence,
        &format!("{label}-rti"),
        &[
            ("BOOMERANG_RTI_BIND", "127.0.0.1:0".into()),
            (
                "BOOMERANG_RTI_READY_ADDRESS",
                ready.local_addr().unwrap().to_string(),
            ),
        ],
    );
    let mut stream = loop {
        match ready.accept() {
            Ok((stream, _)) => break stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(
                    coordinator.child.try_wait().unwrap().is_none(),
                    "RTI exited before readiness: {}",
                    coordinator.output()
                );
                assert!(Instant::now() < deadline, "RTI readiness exceeded deadline");
                std::thread::park_timeout(Duration::from_millis(5));
            }
            Err(error) => panic!("RTI readiness: {error}"),
        }
    };
    // Accepted sockets inherit nonblocking mode on macOS.
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(deadline.saturating_duration_since(Instant::now())))
        .unwrap();
    let mut line = String::new();
    BufReader::new(&mut stream).read_line(&mut line).unwrap();
    let address: SocketAddr = line
        .trim()
        .strip_prefix("BOOMERANG_RTI_READY_V1 ")
        .expect("RTI ready protocol")
        .parse()
        .unwrap();
    assert!(address.ip().is_loopback());
    let env = [("BOOMERANG_RTI_ADDRESS", address.to_string())];
    let mut controller = Process::spawn(
        &artifacts[0],
        &fixture.evidence,
        &format!("{label}-controller"),
        &env,
    );
    let mut sensor = Process::spawn(
        &artifacts[1],
        &fixture.evidence,
        &format!("{label}-sensor"),
        &env,
    );
    let statuses = [
        controller.wait(deadline),
        sensor.wait(deadline),
        coordinator.wait(deadline),
    ];
    let outputs = [controller.output(), sensor.output(), coordinator.output()];
    if scenario == Scenario::Mixed {
        assert!(
            statuses.iter().all(|status| !status.success()),
            "mixed artifacts passed: {statuses:?}\n{outputs:?}"
        );
        assert!(
            outputs[2].contains("Hosted(Wire(Admission(Coordination)))"),
            "missing real handshake rejection: {outputs:?}"
        );
        assert!(!outputs[1].contains("sensor received command 42"));
    } else if scenario == Scenario::RuntimeFailure {
        assert_eq!(statuses[1].code(), Some(42), "{outputs:?}");
        assert!(
            statuses.iter().all(|status| !status.success()),
            "{statuses:?}\n{outputs:?}"
        );
        for output in &outputs {
            assert!(!output.contains("completed"), "{output}");
            assert!(!output.contains("timed out"), "{output}");
        }
    } else {
        assert!(
            statuses.iter().all(ExitStatus::success),
            "{statuses:?}\n{outputs:?}"
        );
        assert!(
            outputs[1].contains("sensor received command 42 at logical 1000000 ns"),
            "{}",
            outputs[1]
        );
        for output in &outputs[..2] {
            assert!(output.contains("member completed"), "{output}");
        }
        assert!(outputs[2].contains("coordinator completed"));
    }
}

pub fn exercise() {
    let fixture = Fixture::new();
    let controller = fixture.build("controller", "original-controller", None, None);
    let sensor = fixture.build("sensor", "original-sensor", None, None);
    let rti = fixture.build("rti", "original-rti", None, None);
    assert_eq!(identity(&controller), identity(&sensor));
    assert_eq!(identity(&controller), identity(&rti));
    run(
        &fixture,
        &[controller.clone(), sensor, rti.clone()],
        Scenario::Matching,
    );
    let failed = fixture.build(
        "sensor",
        "runtime-failure-sensor",
        None,
        Some("runtime-failure"),
    );
    run(
        &fixture,
        &[controller.clone(), failed, rti.clone()],
        Scenario::RuntimeFailure,
    );
    fixture.cargo(
        "test",
        &["-p", "native-vehicle-rti"],
        "generated-rti-contracts",
        None,
    );
    fixture.cargo(
        "test",
        &["-p", "native-vehicle-deployment"],
        "heterogeneous-generation",
        None,
    );
    for role in ["controller", "sensor", "rti"] {
        let package = format!("native-vehicle-{role}");
        let tree = fixture.cargo(
            "tree",
            &[
                "-p",
                &package,
                "--edges",
                "normal,no-proc-macro",
                "--prefix",
                "none",
            ],
            &format!("dependencies-{role}"),
            None,
        );
        for forbidden in [
            "boomerang_builder ",
            "boomerang_codegen ",
            "cargo-boomerang ",
        ] {
            assert!(
                !tree.lines().any(|line| line.starts_with(forbidden)),
                "{role} links {forbidden}: {tree}"
            );
        }
        for (payload, allowed) in [
            ("vehicle-control ", role == "controller"),
            ("sensor-host ", role == "sensor"),
        ] {
            assert_eq!(
                tree.lines().any(|line| line.starts_with(payload)),
                allowed,
                "{role}: {tree}"
            );
        }
    }
    let changed = fixture.build("sensor", "changed-sensor", Some("2000000"), None);
    assert_ne!(identity(&controller), identity(&changed));
    run(&fixture, &[controller, changed, rti], Scenario::Mixed);
}
