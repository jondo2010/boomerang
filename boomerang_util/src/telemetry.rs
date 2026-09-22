//! Hosted observation sampling and bounded UDP telemetry publication.

use std::{
    future::Future,
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::Arc,
    time::{Duration, Instant},
};

use boomerang_runtime::{
    ObservationHandle, ObservationSnapshot, ObservationState, SchedulerLifecycle, SchedulerPhase,
};
use boomerang_telemetry::{
    EncoderError, SchedulerSample, TelemetryEncoder, TelemetryIdentity, MAX_DATAGRAM_BYTES,
};
use tokio::{net::UdpSocket, time::MissedTickBehavior};

/// One hosted scheduler's observation state, monotonic clock, and telemetry encoder.
///
/// Install a clone from [`Self::observation_handle`] in that scheduler's configuration.
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
                scheduler_sample(snapshot),
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
    if period.is_zero() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "telemetry sampling period must be positive",
        ));
    }
    let local_address = SocketAddr::new(
        match destination {
            SocketAddr::V4(_) => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            SocketAddr::V6(_) => IpAddr::V6(Ipv6Addr::UNSPECIFIED),
        },
        0,
    );
    let socket = UdpSocket::bind(local_address).await?;
    socket.writable().await?;
    let mut interval = tokio::time::interval(period);
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut output = [0; MAX_DATAGRAM_BYTES];
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            biased;
            () = &mut shutdown => {
                publish_round(&socket, destination, sources, &mut output);
                return Ok(());
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

/// Converts all runtime observations to absolute portable telemetry values.
///
/// Hosted v1 lifecycle codes are not-started = 0, running = 1, stopped = 2,
/// failed = 3. Phase codes are idle = 0, reaction = 1, framework = 2,
/// physical-wait = 3, external-wait = 4, coordination-wait = 5.
#[must_use]
pub fn scheduler_sample(snapshot: ObservationSnapshot) -> SchedulerSample {
    let ObservationSnapshot {
        lifecycle,
        current_phase,
        current_phase_started_ns,
        reaction_elapsed_ns,
        framework_elapsed_ns,
        physical_wait_elapsed_ns,
        external_wait_elapsed_ns,
        coordination_wait_elapsed_ns,
        processed_tags,
        processed_reactions,
        processed_events,
        set_ports,
        scheduled_actions,
        event_queue_occupancy,
        event_queue_reserved_capacity,
        event_queue_enforced_limit,
        event_queue_peak_occupancy,
        completed_logical_tags,
        last_logical_progress_ns,
    } = snapshot;
    SchedulerSample {
        lifecycle: match lifecycle {
            SchedulerLifecycle::NotStarted => 0,
            SchedulerLifecycle::Running => 1,
            SchedulerLifecycle::Stopped => 2,
            SchedulerLifecycle::Failed => 3,
        },
        current_phase: match current_phase {
            SchedulerPhase::Idle => 0,
            SchedulerPhase::Reaction => 1,
            SchedulerPhase::Framework => 2,
            SchedulerPhase::PhysicalWait => 3,
            SchedulerPhase::ExternalWait => 4,
            SchedulerPhase::CoordinationWait => 5,
        },
        current_phase_started_ns,
        reaction_elapsed_ns,
        framework_elapsed_ns,
        physical_wait_elapsed_ns,
        external_wait_elapsed_ns,
        coordination_wait_elapsed_ns,
        processed_tags,
        processed_reactions,
        processed_events,
        set_ports,
        scheduled_actions,
        event_queue_occupancy,
        event_queue_reserved_capacity,
        event_queue_enforced_limit,
        event_queue_peak_occupancy,
        completed_logical_tags,
        last_logical_progress_ns,
    }
}

#[cfg(test)]
mod tests;
