//! Hosted observation sampling and bounded UDP telemetry publication.
//!
//! Scheduler records carry [`boomerang_runtime::ObservationSnapshot`] directly.
//! Producers and consumers form a closed system deployed atomically from the
//! same build, so this adapter deliberately performs no payload conversion or
//! lifecycle/phase mapping. Snapshot serialization is coupled to the runtime;
//! sampling, encoding, and socket operations stay outside scheduler execution.

use std::{
    future::Future,
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::Arc,
    time::{Duration, Instant},
};

use boomerang_runtime::{ObservationHandle, ObservationState};
use boomerang_telemetry::{EncoderError, TelemetryEncoder, TelemetryIdentity, MAX_DATAGRAM_BYTES};
use tokio::{net::UdpSocket, time::MissedTickBehavior};

/// Owns one hosted UDP telemetry task and the observation handles for one process's schedulers.
///
/// Dropping this guard requests one final best-effort publication before joining the exporter
/// thread. The scheduler never waits on individual UDP sends.
pub struct TelemetryExporter {
    observations: Vec<ObservationHandle>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl TelemetryExporter {
    /// Starts telemetry from the generated launch environment when an endpoint is configured.
    ///
    /// `BOOMERANG_TELEMETRY_ENDPOINT` enables publication. Enabled launchers also require a
    /// shared 32-digit hexadecimal `BOOMERANG_TELEMETRY_RUN_ID`; cadence defaults to 100 ms and
    /// can be overridden with positive integer `BOOMERANG_TELEMETRY_PERIOD_MS`.
    pub fn from_environment(
        artifact_id: [u8; 32],
        process_id: &'static str,
        process_incarnation: u64,
        sources: impl IntoIterator<Item = boomerang_telemetry::SourceIdentity<'static>>,
    ) -> io::Result<Option<Self>> {
        let endpoint = match std::env::var("BOOMERANG_TELEMETRY_ENDPOINT") {
            Ok(value) => value.parse().map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("invalid telemetry endpoint: {error}"),
                )
            })?,
            Err(std::env::VarError::NotPresent) => return Ok(None),
            Err(error) => return Err(io::Error::new(io::ErrorKind::InvalidInput, error)),
        };
        let run_id = std::env::var("BOOMERANG_TELEMETRY_RUN_ID").map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "BOOMERANG_TELEMETRY_RUN_ID is required when telemetry is enabled",
            )
        })?;
        let period = std::env::var("BOOMERANG_TELEMETRY_PERIOD_MS")
            .ok()
            .map(|value| value.parse::<u64>())
            .transpose()
            .map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("invalid telemetry period: {error}"),
                )
            })?
            .unwrap_or(100);
        let run_id = decode_run_id(&run_id)?;
        let identities = sources.into_iter().map(|source| TelemetryIdentity {
            run_id,
            artifact_id,
            process_id,
            process_incarnation,
            source,
        });
        Self::start(identities, endpoint, Duration::from_millis(period)).map(Some)
    }

    /// Starts one Tokio UDP exporter for a fixed set of scheduler sources.
    ///
    /// The identity must borrow static generated metadata because publication outlives launcher
    /// setup until this guard is dropped.
    pub fn start(
        identities: impl IntoIterator<Item = TelemetryIdentity<'static>>,
        destination: SocketAddr,
        period: Duration,
    ) -> io::Result<Self> {
        validate_period(period)?;
        let mut sources = identities
            .into_iter()
            .map(TelemetrySource::new)
            .collect::<Vec<_>>();
        let observations = sources
            .iter()
            .map(TelemetrySource::observation_handle)
            .collect();
        let (shutdown, receiver) = tokio::sync::oneshot::channel();
        let (startup_tx, startup_rx) = std::sync::mpsc::sync_channel(1);
        let thread = std::thread::Builder::new()
            .name(String::from("boomerang-telemetry"))
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_io()
                    .enable_time()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = startup_tx.send(Err(error));
                        return;
                    }
                };
                let socket = match runtime.block_on(bind_udp(destination)) {
                    Ok(socket) => socket,
                    Err(error) => {
                        let _ = startup_tx.send(Err(error));
                        return;
                    }
                };
                if startup_tx.send(Ok(())).is_err() {
                    return;
                }
                runtime.block_on(run_udp_with_socket(
                    socket,
                    &mut sources,
                    destination,
                    period,
                    async move {
                        let _ = receiver.await;
                    },
                ));
            })?;
        match startup_rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                let _ = thread.join();
                return Err(error);
            }
            Err(_) => {
                let _ = thread.join();
                return Err(io::Error::other(
                    "telemetry exporter exited before startup completed",
                ));
            }
        }
        Ok(Self {
            observations,
            shutdown: Some(shutdown),
            thread: Some(thread),
        })
    }

    /// Returns source-order-preserving scheduler observation handles owned by this exporter.
    #[must_use]
    pub fn observation_handles(&self) -> Vec<ObservationHandle> {
        self.observations.iter().map(Arc::clone).collect()
    }
}

fn decode_run_id(value: &str) -> io::Result<[u8; 16]> {
    if value.len() != 32 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "BOOMERANG_TELEMETRY_RUN_ID must contain 32 hexadecimal digits",
        ));
    }
    let mut bytes = [0; 16];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "BOOMERANG_TELEMETRY_RUN_ID must contain 32 hexadecimal digits",
            )
        })?;
    }
    Ok(bytes)
}

impl Drop for TelemetryExporter {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// One hosted scheduler's observation state, monotonic clock, and telemetry encoder.
///
/// Pass a clone from [`Self::observation_handle`] when constructing that scheduler.
/// Constructing the state and its clock together ensures observation fields and record
/// timestamps share one origin. Identity strings remain borrowed from the caller.
pub struct TelemetrySource<'a> {
    origin: Instant,
    observation: ObservationHandle,
    encoder: TelemetryEncoder<'a>,
}

impl<'a> TelemetrySource<'a> {
    /// Creates a fresh observation state and independent record-group sequences.
    #[must_use]
    pub fn new(identity: TelemetryIdentity<'a>) -> Self {
        let origin = Instant::now();
        Self {
            origin,
            observation: Arc::new(ObservationState::new(origin)),
            encoder: TelemetryEncoder::new(identity),
        }
    }

    /// Returns the handle to install on the single scheduler that writes this source.
    #[must_use]
    pub fn observation_handle(&self) -> ObservationHandle {
        Arc::clone(&self.observation)
    }

    /// Samples a scheduler into caller-owned bounded storage.
    ///
    /// Returns `Ok(None)` and increments snapshot misses when the observation state's
    /// bounded coherence retry budget is exhausted. Encoding errors preserve sequence.
    pub fn encode_scheduler(&mut self, output: &mut [u8]) -> Result<Option<usize>, EncoderError> {
        let observed_at = Instant::now();
        let Some(snapshot) = self.observation.snapshot(observed_at) else {
            self.encoder.note_snapshot_miss();
            return Ok(None);
        };
        self.encoder
            .encode_scheduler(
                self.elapsed_ns(Instant::now()),
                self.elapsed_ns(observed_at),
                snapshot,
                output,
            )
            .map(Some)
    }

    /// Encodes this source's cumulative exporter-health counters into bounded storage.
    pub fn encode_exporter_health(&mut self, output: &mut [u8]) -> Result<usize, EncoderError> {
        let now_ns = self.elapsed_ns(Instant::now());
        self.encoder.encode_exporter_health(now_ns, now_ns, output)
    }

    fn elapsed_ns(&self, now: Instant) -> u64 {
        u64::try_from(now.saturating_duration_since(self.origin).as_nanos()).unwrap_or(u64::MAX)
    }
}

/// Samples the fixed source set periodically until `shutdown` completes.
///
/// Run this future on a hosted Tokio executor, independently of scheduler execution.
/// The first round is immediate; missed ticks are skipped. Each round attempts one
/// scheduler record and one health record per source, using one reusable bounded
/// buffer and nonblocking UDP sends. There is no publication queue or retry backlog.
/// Encoding and send failures count as publication drops and never mutate scheduler
/// state. A failed snapshot is skipped and counted separately.
///
/// Shutdown attempts one final round and returns without waiting for delivery. Arrange
/// for scheduler finalization before completing `shutdown` to include final lifecycle
/// state. Dropping this future instead cancels it without a final publication.
///
/// # Errors
///
/// Returns an error for a zero period or failure to initialize the local UDP socket.
/// UDP delivery is best effort and remote receipt is not acknowledged.
pub async fn run_udp(
    sources: &mut [TelemetrySource<'_>],
    destination: SocketAddr,
    period: Duration,
    shutdown: impl Future<Output = ()>,
) -> io::Result<()> {
    validate_period(period)?;
    let socket = bind_udp(destination).await?;
    run_udp_with_socket(socket, sources, destination, period, shutdown).await;
    Ok(())
}

fn validate_period(period: Duration) -> io::Result<()> {
    if period.is_zero() {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "telemetry sampling period must be positive",
        ))
    } else {
        Ok(())
    }
}

async fn bind_udp(destination: SocketAddr) -> io::Result<UdpSocket> {
    let local_address = SocketAddr::new(
        match destination {
            SocketAddr::V4(_) => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            SocketAddr::V6(_) => IpAddr::V6(Ipv6Addr::UNSPECIFIED),
        },
        0,
    );
    let socket = UdpSocket::bind(local_address).await?;
    socket.writable().await?;
    Ok(socket)
}

async fn run_udp_with_socket(
    socket: UdpSocket,
    sources: &mut [TelemetrySource<'_>],
    destination: SocketAddr,
    period: Duration,
    shutdown: impl Future<Output = ()>,
) {
    let mut interval = tokio::time::interval(period);
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut output = [0; MAX_DATAGRAM_BYTES];
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            biased;
            () = &mut shutdown => {
                publish_round(&socket, destination, sources, &mut output);
                return;
            }
            _ = interval.tick() => publish_round(&socket, destination, sources, &mut output),
        }
    }
}

fn publish_round(
    socket: &UdpSocket,
    destination: SocketAddr,
    sources: &mut [TelemetrySource<'_>],
    output: &mut [u8; MAX_DATAGRAM_BYTES],
) {
    for source in sources {
        match source.encode_scheduler(output) {
            Ok(Some(length)) => publish(socket, destination, source, &output[..length]),
            Ok(None) => {}
            Err(_) => source.encoder.note_publication_drop(),
        }
        match source.encode_exporter_health(output) {
            Ok(length) => publish(socket, destination, source, &output[..length]),
            Err(_) => source.encoder.note_publication_drop(),
        }
    }
}

fn publish(
    socket: &UdpSocket,
    destination: SocketAddr,
    source: &mut TelemetrySource<'_>,
    bytes: &[u8],
) {
    if socket.try_send_to(bytes, destination).ok() != Some(bytes.len()) {
        source.encoder.note_publication_drop();
    }
}

#[cfg(test)]
mod tests;
