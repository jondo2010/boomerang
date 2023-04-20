use std::net::SocketAddr;

use futures::{sink::SinkExt, StreamExt};
use tinymap::Key;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};
use tokio_util::codec::Framed;

use crate::{
    bincodec, ClockSyncStat, Error, FederateKey, NeighborStructure, RejectReason, RtiMsg, Tag,
    Timestamp,
};

mod federate;
pub(crate) mod start_time_sync;
pub use federate::*;

/// Mode of execution of a federate.
pub enum ExecutionMode {
    Fast,
    RealTime,
}

type InitialMap = tinymap::TinySecondaryMap<
    FederateKey,
    (
        Framed<TcpStream, bincodec::BinCodec<RtiMsg, bincode::DefaultOptions>>,
        NeighborStructure,
        ClockSyncStat,
    ),
>;

/// Structure that an RTI instance uses to keep track of its own and its corresponding federates' state.
#[derive(Debug)]
pub struct Rti {
    // Condition variable used to signal receipt of all proposed start times.
    //pthread_cond_t received_start_times;

    // Condition variable used to signal that a start time has been sent to a federate.
    //pthread_cond_t sent_start_time;
    /// RTI's decided stop tag for federates
    max_stop_tag: Option<Tag>,

    /// Number of federates in the federation
    number_of_federates: usize,

    // The federates.
    //federate_t* federates;
    /// Number of federates handling stop
    num_feds_handling_stop: usize,

    /// Boolean indicating that all federates have exited.
    all_federates_exited: bool,

    /// The ID of the federation that this RTI will supervise.
    ///
    /// This should be overridden with a command-line -i option to ensure that each federate only
    /// joins its assigned federation.
    federation_id: String,

    /// The desired port specified by the user on the command line.
    user_specified_port: u16,
    /// The final port number that the TCP socket server ends up using.
    final_port_TCP: u16,
    /// The final port number that the UDP socket server ends up using. */
    final_port_UDP: u16,

    // Clock synchronization information
    /// Indicates whether clock sync is globally on for the federation. Federates can still
    /// selectively disable clock synchronization if they wanted to.
    clock_sync_global_status: ClockSyncStat,

    /// Frequency (period in nanoseconds) between clock sync attempts.
    clock_sync_period_ns: u64,

    /// Number of messages exchanged for each clock sync attempt.
    clock_sync_exchanges_per_interval: usize,

    /// Boolean indicating that authentication is enabled.
    authentication_enabled: bool,
}

pub struct RtiHandles {
    pub start_time: Timestamp,
    pub federate_handles: Vec<JoinHandle<()>>,
    pub listener_handle: JoinHandle<()>,
}

impl Rti {
    pub fn new(number_of_federates: usize, federation_id: &str) -> Self {
        Self {
            max_stop_tag: None,
            number_of_federates,
            num_feds_handling_stop: 0,
            all_federates_exited: false,
            federation_id: federation_id.to_string(),
            user_specified_port: 0,
            final_port_TCP: 0,
            final_port_UDP: 0,
            clock_sync_global_status: ClockSyncStat::Off,
            clock_sync_period_ns: 0,
            clock_sync_exchanges_per_interval: 0,
            authentication_enabled: false,
        }
    }

    pub async fn create_listener(&self, port: u16) -> Result<TcpListener, Error> {
        tracing::info!(
            "RTI using port {port} for federation {}",
            self.federation_id,
        );

        let addr = SocketAddr::new("127.0.0.1".parse().unwrap(), port);
        TcpListener::bind(&addr)
            .await
            .map_err(|err| Error::Other(err.into()))
    }

    /// Start a TCP server that listens for incoming connections from federates on localhost.
    pub async fn start_server(&mut self, listener: TcpListener) -> Result<RtiHandles, Error> {
        let number_of_federates = self.number_of_federates;
        let federation_id = self.federation_id.clone();
        let clock_sync_global_status = self.clock_sync_global_status;

        let initial = initialize_federates(
            number_of_federates,
            federation_id,
            clock_sync_global_status,
            &listener,
        )
        .await?;

        // For each federate, create a channel to send `RtiMsg` to it.
        let (tx_map, rx_map): (
            tinymap::TinySecondaryMap<FederateKey, _>,
            tinymap::TinySecondaryMap<FederateKey, _>,
        ) = initial
            .into_iter()
            .map(|(federate_id, (frame, neighbors, clock_sync))| {
                let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<RtiMsg>();
                (
                    (federate_id, tx),
                    (federate_id, (frame, rx, neighbors, clock_sync)),
                )
            })
            .unzip();

        // Start time synchronizer
        let (start_time_sync, synchronizer) = start_time_sync::create(self.number_of_federates);
        let start_time_handle = tokio::spawn(synchronizer.negotiate_start_time());

        let federate_handles = rx_map
            .into_iter()
            .map(|(federate_id, (frame, rx, neighbors, clock_sync))| {
                // Create a tasks to communicate with the federates.
                tokio::spawn(
                    Federate::new(
                        federate_id,
                        start_time_sync.clone(),
                        clock_sync,
                        neighbors,
                        rx,
                        tx_map.clone(),
                    )
                    .run(frame),
                )
            })
            .collect::<Vec<_>>();

        // All federates have connected.
        tracing::debug!("All federates have connected to RTI.");

        // Any additional connections are errors. Respond to them in a separate task.
        let listener_handle = tokio::spawn(erroneous_connections(listener));

        let start_time = start_time_handle.await.map_err(|err| {
            tracing::error!("RTI failed to negotiate start time: {}", err);
            Error::Other(err.into())
        })?;

        Ok(RtiHandles {
            start_time,
            federate_handles,
            listener_handle,
        })
    }
}

/// Listen for up to `number_of_federates` connections from federates, returning initial handshaking data.
async fn initialize_federates(
    number_of_federates: usize,
    federation_id: String,
    clock_sync_global_status: ClockSyncStat,
    listener: &TcpListener,
) -> Result<InitialMap, Error> {
    let mut initial = InitialMap::new();
    while initial.len() < number_of_federates {
        let (socket, _) = listener.accept().await.map_err(|err| {
            tracing::error!("RTI failed to accept connection: {}", err);
            Error::Other(err.into())
        })?;

        tracing::info!("Got connection from {:?}", socket.peer_addr());

        let mut frame = Framed::new(socket, bincodec::create::<RtiMsg>());

        match connect_to_federates(
            &federation_id,
            number_of_federates,
            initial.keys(),
            clock_sync_global_status,
            &mut frame,
        )
        .await
        {
            Ok((federate_id, neighbors, clock_sync)) => {
                // Federate ID and federation ID are correct. Send back an `Accept` message.
                tracing::info!("RTI accepted federate.");
                initial.insert(federate_id, (frame, neighbors, clock_sync));
            }
            Err(Error::Reject(reason)) => {
                // Federate ID and federation ID are incorrect. Send back a `Reject` message.
                tracing::warn!("RTI rejected federate.");
                frame
                    .send(RtiMsg::Reject(reason))
                    .await
                    .map_err(|err| Error::Other(err.into()))?;
                frame
                    .close()
                    .await
                    .map_err(|err| Error::Other(err.into()))?;
            }
            Err(err) => {
                tracing::warn!("Unexpected error negotiating with federate: {err:?}");
                frame
                    .close()
                    .await
                    .map_err(|err| Error::Other(err.into()))?;
            }
        }
    }

    Ok(initial)
}

async fn connect_to_federates<T>(
    federation_id: &str,
    number_of_federates: usize,
    seen_federate_ids: impl Iterator<Item = FederateKey>,
    clock_sync_global_status: ClockSyncStat,
    frame: &mut Framed<T, bincodec::BinCodec<RtiMsg, bincode::DefaultOptions>>,
) -> Result<(FederateKey, NeighborStructure, ClockSyncStat), Error>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    let federate_id =
        check_fed_ids(federation_id, number_of_federates, seen_federate_ids, frame).await?;

    tracing::debug!("RTI responding with `Ack` to federate {federate_id:?}.");
    frame.send(RtiMsg::Ack).await.map_err(|err| {
        tracing::error!("RTI failed to send `Ack` to federate: {}", err);
        Error::Other(err.into())
    })?;

    let neighbors = receive_connection_information(frame).await?;
    let clock_sync = set_up_clock_sync(clock_sync_global_status, frame).await?;

    Ok((federate_id, neighbors, clock_sync))
}

/// The first message from the federate should contain its ID and the federation ID.
#[tracing::instrument(level = "debug", skip(frame, seen_federate_ids))]
async fn check_fed_ids<T>(
    federation_id: &str,
    number_of_federates: usize,
    mut seen_federate_ids: impl Iterator<Item = FederateKey>,
    frame: &mut Framed<T, bincodec::BinCodec<RtiMsg, bincode::DefaultOptions>>,
) -> Result<FederateKey, RejectReason>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    let msg = match frame.next().await {
        Some(Ok(msg)) => Ok(msg),
        _ => {
            tracing::warn!("RTI did not receive a message from federate.");
            Err(RejectReason::UnexpectedMessage)
        }
    }?;

    match msg {
        RtiMsg::FedIds(fed_ids) => {
            tracing::debug!(?fed_ids, "RTI received FedIds.");

            // Compare the received federation ID to mine.
            if federation_id != fed_ids.federation {
                // Federation IDs do not match. Send back a `Reject` message.
                tracing::warn!(
                            "Federate from another federation {} attempted to connect to RTI in federation {}.",
                            fed_ids.federation,
                            federation_id
                        );
                Err(RejectReason::FederationIdDoesNotMatch)
            } else if fed_ids.federate_key.index() >= number_of_federates {
                // Federate ID is out of range.
                tracing::error!(?fed_ids.federate_key, "RTI received federate ID out of range.");
                Err(RejectReason::FederateKeyOutOfRange)
            } else if seen_federate_ids
                .find(|&seen_federate_id| seen_federate_id == fed_ids.federate_key)
                .is_some()
            {
                // Federate ID has already been seen.
                tracing::error!(?fed_ids.federate_key, "RTI received duplicate federate ID.");
                Err(RejectReason::FederateKeyInUse)
            } else {
                Ok(fed_ids.federate_key)
            }
        }
        RtiMsg::P2PSendingFedId(..) | RtiMsg::P2PTaggedMessage(..) => {
            // The federate is trying to connect to a peer, not to the RTI. It has connected to the RTI instead.
            // FIXME: This should not happen, but apparently has been observed.
            // It should not happen because the peers get the port and IP address of the peer they want to connect to from the RTI.  If the connection is a peer-to-peer connection between two federates, reject the connection with the WRONG_SERVER error.
            tracing::warn!(received = ?msg, "RTI expected a `FedIds` message.");
            Err(RejectReason::WrongServer)
        }
        _ => {
            tracing::warn!(received = ?msg, "RTI expected a `FedIds` message.");
            Err(RejectReason::UnexpectedMessage)
        }
    }
}

/// The second message from the federate should contain its neighbor structure.
#[tracing::instrument(level = "debug", skip(frame))]
async fn receive_connection_information<T>(
    frame: &mut Framed<T, bincodec::BinCodec<RtiMsg, bincode::DefaultOptions>>,
) -> Result<NeighborStructure, RejectReason>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    let msg = match frame.next().await {
        Some(Ok(msg)) => Ok(msg),
        _ => {
            tracing::warn!("RTI did not receive a message from federate.");
            Err(RejectReason::UnexpectedMessage)
        }
    }?;

    match msg {
        RtiMsg::NeighborStructure(neighbor_structure) => {
            tracing::debug!(?neighbor_structure, "RTI received neighbor structure.");
            Ok(neighbor_structure)
        }
        _ => {
            tracing::warn!(received = ?msg, "RTI expected a `NeighborStructure` message.");
            Err(RejectReason::UnexpectedMessage)
        }
    }
}

/// Read the `UdpPort` message from the federate regardless of the status of clock synchronization.
/// This message will tell the RTI whether the federate is doing clock synchronization, and if
/// it is, what port to use for UDP.
#[tracing::instrument(level = "debug", skip(frame))]
async fn set_up_clock_sync<T>(
    clock_sync_global_status: ClockSyncStat,
    frame: &mut Framed<T, bincodec::BinCodec<RtiMsg, bincode::DefaultOptions>>,
) -> Result<ClockSyncStat, RejectReason>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    let msg = match frame.next().await {
        Some(Ok(msg)) => Ok(msg),
        _ => {
            tracing::warn!("RTI did not receive a message from federate.");
            Err(RejectReason::UnexpectedMessage)
        }
    }?;

    match msg {
        RtiMsg::UdpPort(clock_sync) => {
            tracing::debug!(?clock_sync, "RTI received UdpPort.");

            if clock_sync_global_status.is_on() && clock_sync.is_on() {
                // Perform the initialization clock synchronization with the federate.
                // Send the required number of messages for clock synchronization
                todo!("Clock synchronization is not yet implemented.");
            } else {
                // No clock synchronization at all.
                // Clock synchronization is universally disabled via the clock-sync command-line parameter
                // (-c off was passed to the RTI).
                // Note that the federates are still going to send a MSG_TYPE_UDP_PORT message but with a payload (port) of -1.
                //_RTI.federates[fed_id].clock_synchronization_enabled = false;
                return Ok(clock_sync);
            }
        }
        _ => {
            tracing::warn!(received = ?msg, "RTI expected a `UdpPort` message.");
            Err(RejectReason::UnexpectedMessage)
        }
    }
}

async fn erroneous_connections(listener: TcpListener) {
    loop {
        match listener.accept().await {
            Ok((socket, addr)) => {
                tracing::warn!(
                    ?addr,
                    "RTI received an unexpected connection request. Federation is running."
                );
                let mut f = Framed::new(socket, bincodec::create::<RtiMsg>());
                f.send(RtiMsg::Reject(RejectReason::FederationIdDoesNotMatch))
                    .await
                    .unwrap();
                f.close().await.unwrap();
            }
            Err(_) => {
                tracing::error!("RTI failed to accept a connection from a federate.");
            }
        }
    }
}
