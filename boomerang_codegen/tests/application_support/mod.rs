use std::{
    fs::{self, File},
    io::{BufRead, BufReader},
    net::{SocketAddr, TcpListener},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    time::{Duration, Instant},
};

const DEADLINE: Duration = Duration::from_secs(20);

/// Remove application configuration inherited from the test environment.
fn clean_environment(command: &mut Command) {
    for (name, _) in std::env::vars_os() {
        let key = name.to_string_lossy();
        if key.starts_with("BOOMERANG_") || key.starts_with("NATIVE_VEHICLE_") {
            command.env_remove(name);
        }
    }
}

fn artifact(directory: &Path, name: &str) -> PathBuf {
    let path = directory.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    assert!(
        path.is_file(),
        "missing prebuilt executable: {}",
        path.display()
    );
    path
}

/// Exercise both ordinary input and source admission through prebuilt applications.
pub fn local(directory: &Path) {
    for name in ["consumer", "consumer-source"] {
        let mut process = Process::spawn(&artifact(directory, name), directory, name, &[]);
        let status = process.wait(Instant::now() + DEADLINE);
        let output = process.output();
        assert!(status.success(), "{output}");
        assert!(
            output.contains("connected routing and initialized input"),
            "{output}"
        );
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

fn run(evidence: &Path, artifacts: &[PathBuf; 3], scenario: Scenario) {
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
        evidence,
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
        evidence,
        &format!("{label}-controller"),
        &env,
    );
    let mut sensor = Process::spawn(&artifacts[1], evidence, &format!("{label}-sensor"), &env);
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

pub fn federation(directory: &Path) {
    let controller = artifact(directory, "controller");
    let sensor = artifact(directory, "sensor");
    let rti = artifact(directory, "rti");
    assert_eq!(identity(&controller), identity(&sensor));
    assert_eq!(identity(&controller), identity(&rti));
    run(
        directory,
        &[controller.clone(), sensor, rti.clone()],
        Scenario::Matching,
    );
    run(
        directory,
        &[
            controller.clone(),
            artifact(directory, "sensor-failure"),
            rti.clone(),
        ],
        Scenario::RuntimeFailure,
    );
    let changed = artifact(directory, "sensor-mixed");
    assert_ne!(identity(&controller), identity(&changed));
    run(directory, &[controller, changed, rti], Scenario::Mixed);
}
