//! Scoped ownership and bounded startup/termination of generated host processes.

use anyhow::{anyhow, bail, Context, Result};
use std::{
    io::{self, Read},
    net::{SocketAddr, TcpListener},
    process::{Child, Command, ExitStatus},
    thread,
    time::{Duration, Instant},
};

/// Poll interval keeps failure detection responsive without spinning.
const POLL: Duration = Duration::from_millis(10);

/// Owns every spawned child until its exit status has been collected.
#[derive(Default)]
pub(super) struct Processes {
    /// Children remain owned on all error paths, including partial startup.
    children: Vec<Child>,
}

impl Processes {
    /// Starts and retains one child before returning to fallible caller code.
    pub(super) fn spawn(&mut self, command: &mut Command) -> Result<()> {
        self.children.push(
            command
                .spawn()
                .context("failed to start generated process")?,
        );
        Ok(())
    }

    /// Starts the RTI and receives its bounded readiness announcement.
    pub(super) fn start_rti(
        &mut self,
        command: &mut Command,
        timeout: Duration,
    ) -> Result<SocketAddr> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        command.env(
            "BOOMERANG_RTI_READY_ADDRESS",
            listener.local_addr()?.to_string(),
        );
        self.spawn(command)?;
        let child = self.children.last_mut().expect("just spawned RTI");
        let deadline = Instant::now() + timeout;
        let mut stream = None;
        let mut bytes = Vec::new();
        loop {
            if Instant::now() >= deadline {
                bail!("RTI readiness timed out");
            }
            if let Some(exited) = child.try_wait()? {
                bail!("RTI exited before readiness: {exited}");
            }
            if stream.is_none() {
                match listener.accept() {
                    Ok((accepted, _)) => {
                        accepted.set_nonblocking(true)?;
                        stream = Some(accepted);
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                    Err(error) => return Err(error.into()),
                }
            }
            if let Some(stream) = &mut stream {
                let mut buffer = [0; 128];
                match stream.read(&mut buffer) {
                    Ok(0) => bail!("RTI closed readiness before a complete announcement"),
                    Ok(len) => bytes.extend_from_slice(&buffer[..len]),
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                    Err(error) => return Err(error.into()),
                }
                if bytes.len() > 128 {
                    bail!("oversized RTI readiness announcement");
                }
                if bytes.ends_with(b"\n") {
                    break;
                }
            }
            thread::sleep(POLL);
        }
        let line = std::str::from_utf8(&bytes)?;
        let address: SocketAddr = line
            .strip_prefix("BOOMERANG_RTI_READY_V1 ")
            .ok_or_else(|| anyhow!("invalid RTI readiness announcement"))?
            .trim()
            .parse()?;
        if !address.ip().is_loopback() || address.port() == 0 {
            bail!("RTI readiness must identify a bound loopback address");
        }
        Ok(address)
    }

    /// Collects every status, bounding peer shutdown after the first child exits.
    pub(super) fn wait(&mut self, shutdown: Duration) -> Result<ExitStatus> {
        let mut statuses = vec![None; self.children.len()];
        let mut deadline = None;
        let mut failure = None;
        loop {
            for (child, status) in self.children.iter_mut().zip(&mut statuses) {
                if status.is_some() {
                    continue;
                }
                if let Some(exited) = child.try_wait()? {
                    *status = Some(exited);
                    deadline.get_or_insert_with(|| Instant::now() + shutdown);
                    if !exited.success() && failure.is_none() {
                        failure = Some(exited);
                        // Give the RTI a brief opportunity to deliver its diagnostic to peers.
                        deadline = Some(Instant::now() + shutdown.min(Duration::from_secs(1)));
                    }
                }
            }
            if statuses.iter().all(Option::is_some) {
                return failure
                    .or_else(|| statuses.first().copied().flatten())
                    .ok_or_else(|| anyhow!("no generated processes were started"));
            }
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                self.terminate();
                return failure
                    .map(Ok)
                    .unwrap_or_else(|| Err(anyhow!("generated process shutdown timed out")));
            }
            thread::sleep(POLL);
        }
    }

    /// Kills outstanding peers together, then reaps every retained process.
    fn terminate(&mut self) {
        for child in &mut self.children {
            if !matches!(child.try_wait(), Ok(Some(_))) {
                let _ = child.kill();
            }
        }
        for child in &mut self.children {
            let _ = child.wait();
        }
    }
}

impl Drop for Processes {
    fn drop(&mut self) {
        self.terminate();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write, net::TcpStream};

    /// Gives a fixture ample startup time while keeping negative cases bounded.
    const STARTUP_BOUND: Duration = Duration::from_secs(2);
    /// Bounds observation of child termination independently from the supervisor.
    const OBSERVATION_BOUND: Duration = Duration::from_secs(5);
    /// Isolated current-executable fixture selected through the ignored test harness.
    const FIXTURE: &str = "run::processes::tests::process_fixture";

    /// Runs only in a child process; its observer socket proves startup and termination.
    #[test]
    #[ignore = "subprocess fixture invoked explicitly by process supervisor tests"]
    fn process_fixture() {
        let mode = std::env::var("BOOMERANG_TEST_PROCESS_MODE").expect("fixture mode");
        let mut observer = TcpStream::connect(
            std::env::var("BOOMERANG_TEST_PROCESS_OBSERVER").expect("fixture observer"),
        )
        .unwrap();
        observer.write_all(b"S").unwrap();
        if mode == "failure" {
            std::process::exit(17);
        }
        if mode == "success" {
            std::process::exit(0);
        }
        let mut readiness = None;
        let mut endpoint = None;
        if matches!(mode.as_str(), "malformed" | "partial" | "ready") {
            let mut stream = TcpStream::connect(
                std::env::var("BOOMERANG_RTI_READY_ADDRESS").expect("readiness address"),
            )
            .unwrap();
            match mode.as_str() {
                "malformed" => stream.write_all(b"INVALID_READY\n").unwrap(),
                "partial" => stream
                    .write_all(b"BOOMERANG_RTI_READY_V1 127.0.0.1:1")
                    .unwrap(),
                "ready" => {
                    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
                    writeln!(
                        stream,
                        "BOOMERANG_RTI_READY_V1 {}",
                        listener.local_addr().unwrap()
                    )
                    .unwrap();
                    endpoint = Some(listener);
                }
                _ => unreachable!(),
            }
            readiness = Some(stream);
        }
        // Block on an explicit parent signal, keeping partial readiness open.
        let mut signal = [0];
        let _ = observer.read(&mut signal);
        drop((readiness, endpoint));
    }

    /// Creates a child invocation and the socket used to observe its lifetime.
    fn fixture(mode: &str) -> (Command, TcpListener) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                FIXTURE,
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("BOOMERANG_TEST_PROCESS_MODE", mode)
            .env(
                "BOOMERANG_TEST_PROCESS_OBSERVER",
                listener.local_addr().unwrap().to_string(),
            )
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        (command, listener)
    }

    /// Waits for a fixture's startup signal under an independent observation deadline.
    fn observe(listener: &TcpListener) -> TcpStream {
        let deadline = Instant::now() + OBSERVATION_BOUND;
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_nonblocking(false).unwrap();
                    stream.set_read_timeout(Some(OBSERVATION_BOUND)).unwrap();
                    let mut started = [0];
                    stream.read_exact(&mut started).unwrap();
                    assert_eq!(started, *b"S");
                    return stream;
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "fixture did not report startup");
                    // Poll cadence only; no test behavior depends on an arbitrary delay.
                    thread::sleep(POLL);
                }
                Err(error) => panic!("fixture observation failed: {error}"),
            }
        }
    }

    /// Confirms that a blocked fixture no longer owns its observation connection.
    fn assert_terminated(mut observer: TcpStream) {
        let mut signal = [0];
        match observer.read(&mut signal) {
            Ok(0) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionAborted
                ) => {}
            result => panic!("fixture was not terminated: {result:?}"),
        }
    }

    #[test]
    fn missing_and_partial_readiness_time_out_and_drop_reaps_children() {
        for mode in ["missing", "partial"] {
            let (mut command, listener) = fixture(mode);
            let mut processes = Processes::default();
            let started = Instant::now();
            let error = processes
                .start_rti(&mut command, STARTUP_BOUND)
                .unwrap_err();
            assert!(
                error.to_string().contains("readiness timed out"),
                "{error:#}"
            );
            assert!(started.elapsed() < OBSERVATION_BOUND);
            let observer = observe(&listener);
            processes.terminate();
            assert!(processes
                .children
                .iter_mut()
                .all(|child| child.try_wait().unwrap().is_some()));
            drop(processes);
            assert_terminated(observer);
        }
    }

    #[test]
    fn malformed_readiness_is_rejected_and_scope_cleanup_stops_child() {
        let (mut command, listener) = fixture("malformed");
        let observer;
        {
            let mut processes = Processes::default();
            let error = processes
                .start_rti(&mut command, STARTUP_BOUND)
                .unwrap_err();
            assert!(
                error.to_string().contains("invalid RTI readiness"),
                "{error:#}"
            );
            observer = observe(&listener);
        }
        assert_terminated(observer);
    }

    #[test]
    fn valid_readiness_accepts_live_endpoint_and_drop_stops_child() {
        let (mut command, listener) = fixture("ready");
        let observer;
        {
            let mut processes = Processes::default();
            let address = processes.start_rti(&mut command, STARTUP_BOUND).unwrap();
            observer = observe(&listener);
            assert!(address.ip().is_loopback());
            TcpStream::connect_timeout(&address, OBSERVATION_BOUND).unwrap();
        }
        assert_terminated(observer);
    }

    #[test]
    fn peer_failure_is_preserved_and_blocked_peer_is_killed_and_reaped() {
        let (mut blocked, blocked_listener) = fixture("blocked");
        let (mut failing, failed_listener) = fixture("failure");
        let mut processes = Processes::default();
        processes.spawn(&mut blocked).unwrap();
        let blocked_observer = observe(&blocked_listener);
        processes.spawn(&mut failing).unwrap();
        let failed_observer = observe(&failed_listener);
        let status = processes.wait(Duration::from_millis(50)).unwrap();
        assert_eq!(status.code(), Some(17));
        assert!(processes
            .children
            .iter_mut()
            .all(|child| child.try_wait().unwrap().is_some()));
        assert_terminated(blocked_observer);
        assert_terminated(failed_observer);
    }

    #[test]
    fn successful_peer_exit_bounds_shutdown_of_blocked_peer() {
        let (mut blocked, blocked_listener) = fixture("blocked");
        let (mut successful, successful_listener) = fixture("success");
        let mut processes = Processes::default();
        processes.spawn(&mut blocked).unwrap();
        let observer = observe(&blocked_listener);
        processes.spawn(&mut successful).unwrap();
        let _successful_observer = observe(&successful_listener);
        let error = processes.wait(Duration::from_millis(50)).unwrap_err();
        assert!(error.to_string().contains("shutdown timed out"));
        assert!(processes
            .children
            .iter_mut()
            .all(|child| child.try_wait().unwrap().is_some()));
        assert_terminated(observer);
    }

    #[test]
    fn spawn_error_unwinds_and_cleans_up_already_started_child() {
        let (mut blocked, listener) = fixture("blocked");
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("missing-executable");
        let observer;
        {
            let mut processes = Processes::default();
            processes.spawn(&mut blocked).unwrap();
            observer = observe(&listener);
            let error = processes.spawn(&mut Command::new(missing)).unwrap_err();
            assert!(error
                .to_string()
                .contains("failed to start generated process"));
        }
        assert_terminated(observer);
    }
}
