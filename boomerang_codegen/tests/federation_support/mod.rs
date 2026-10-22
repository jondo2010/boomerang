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
        let target = root.join("../target/cargo-native-federation-slice4");
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

    fn build(&self, role: &str, label: &str, delay: Option<&str>) -> PathBuf {
        let package = format!("native-vehicle-{role}");
        let output = self.cargo(
            "build",
            &["-p", &package, "--message-format=json-render-diagnostics"],
            label,
            delay,
        );
        let source = output
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .find(|value| {
                value["reason"] == "build-script-executed"
                    && value["package_id"]
                        .as_str()
                        .unwrap_or("")
                        .contains(&package)
            })
            .and_then(|value| value["out_dir"].as_str().map(PathBuf::from))
            .expect("Cargo reports the application build script output");
        let generated = fs::read_to_string(source.join("boomerang.rs")).unwrap();
        fs::write(self.evidence.join(format!("{label}.rs")), &generated).unwrap();
        let (local, remote) = match role {
            "controller" => (Some("vehicle_control::"), Some("sensor_host::")),
            "sensor" => (Some("sensor_host::"), Some("vehicle_control::")),
            "rti" => (None, None),
            _ => unreachable!(),
        };
        if let Some(local) = local {
            assert!(generated.contains(local), "missing local payload: {label}");
        }
        if let Some(remote) = remote {
            assert!(!generated.contains(remote), "remote payload: {label}");
        }
        if role == "rti" {
            assert!(!generated.contains("sensor_host::"));
            assert!(!generated.contains("vehicle_control::"));
            assert!(!generated.contains("generated_bindings"));
        }
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

fn clean_environment(command: &mut Command) {
    for (key, _) in std::env::vars_os() {
        let name = key.to_string_lossy();
        if name.starts_with("BOOMERANG_")
            || name.starts_with("NATIVE_VEHICLE_")
            || matches!(
                name.as_ref(),
                "RUSTC_WRAPPER"
                    | "RUSTC_WORKSPACE_WRAPPER"
                    | "RUSTFLAGS"
                    | "CARGO_ENCODED_RUSTFLAGS"
            )
        {
            command.env_remove(key);
        }
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

fn run(fixture: &Fixture, artifacts: &[PathBuf; 3], mixed: bool) {
    let label = if mixed { "mixed" } else { "matching" };
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
    if mixed {
        assert!(
            statuses.iter().all(|status| !status.success()),
            "mixed artifacts passed: {statuses:?}\n{outputs:?}"
        );
        assert!(
            outputs[2].contains("Hosted(Wire(Admission(Coordination)))"),
            "missing real handshake rejection: {outputs:?}"
        );
        assert!(!outputs[1].contains("sensor received command 42"));
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
    let controller = fixture.build("controller", "original-controller", None);
    let sensor = fixture.build("sensor", "original-sensor", None);
    let rti = fixture.build("rti", "original-rti", None);
    assert_eq!(identity(&controller), identity(&sensor));
    assert_eq!(identity(&controller), identity(&rti));
    let rebuilt = fixture.build("controller", "unchanged-controller", None);
    assert_eq!(identity(&controller), identity(&rebuilt));
    run(&fixture, &[controller.clone(), sensor, rti.clone()], false);
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
    let changed = fixture.build("sensor", "changed-sensor", Some("2000000"));
    assert_ne!(identity(&controller), identity(&changed));
    run(&fixture, &[controller, changed, rti], true);
}
