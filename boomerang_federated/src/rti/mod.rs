use std::{net::SocketAddr, time::Duration};

use futures::{sink::SinkExt, StreamExt};
use tinymap::Key;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::{TcpListener, TcpStream},
    sync::mpsc,
};
use tokio_stream::wrappers::UnboundedReceiverStream;
use tokio_util::codec::Framed;

use crate::{
    util::{bincodec, mpsc_sink::UnboundedSenderSink},
    ClockSyncStat, Error, FederateKey, NeighborStructure, RejectReason, RtiMsg,
};

mod federate;
use federate::*;

pub(crate) mod start_time_sync;

/// Mode of execution of a federate.
pub enum ExecutionMode {
    Fast,
    RealTime,
}

/// Configuration for the RTI.
pub struct Config {
    /// The ID of the federation that this RTI will supervise.
    federation_id: String,
    /// Number of federates in the federation
    number_of_federates: usize,
    /// Indicates whether clock sync is globally on for the federation. Federates can still selectively disable.
    clock_sync_global_status: ClockSyncStat,
    /// Boolean indicating that authentication is enabled.
    authentication_enabled: bool,
}

impl Config {
    pub fn new(federation_id: impl Into<String>) -> Self {
        Self {
            federation_id: federation_id.into(),
            number_of_federates: 0,
            clock_sync_global_status: ClockSyncStat::Init,
            authentication_enabled: false,
        }
    }

    pub fn with_federates(self, number_of_federates: usize) -> Self {
        Self {
            number_of_federates,
            ..self
        }
    }
}

/// Start a TCP server that listens for incoming connections from federates on localhost.
pub async fn create_listener(port: u16) -> Result<TcpListener, Error> {
    tracing::info!("RTI listening on port {port}",);
    let addr = SocketAddr::new("127.0.0.1".parse().unwrap(), port);
    TcpListener::bind(&addr)
        .await
        .map_err(|err| Error::Other(err.into()))
}

/// Start the RTI given a TCP listener and config.
#[tracing::instrument(skip(listener, config), fields(federation_id=?config.federation_id))]
pub async fn start_rti(listener: TcpListener, config: Config) -> Result<RtiHandles, Error> {
    // Initialize all federates
    let init_federates = initialize_federates(
        config.number_of_federates,
        config.federation_id,
        config.clock_sync_global_status,
        &listener,
    )
    .await?;

    let upstream_delay_map = build_transitive_upstream_delay_map(
        init_federates
            .iter()
            .map(|(key, fed)| (key, &fed.neighbors.upstream)),
    );

    // Any additional connections are errors. Respond to them in a separate task.
    let listener_handle = tokio::spawn(erroneous_connections(listener));

    let (start_time_sync, synchronizer) = start_time_sync::create(config.number_of_federates);
    let start_time_handle = tokio::spawn(synchronizer.negotiate_start_time());

    // Split the frame from each federate connection into a stream and a sink.
    let mut federate_states = tinymap::TinySecondaryMap::new();
    let federate_streams = init_federates.into_iter().map(|(federate_key, initial)| {
        // Split the frame into a stream and a sink.
        let (frame_sink, frame_stream) = initial.frame.split();

        // Wrap the sink in an unbounded channel so it can clone
        let sink = {
            let (sender, receiver) = mpsc::unbounded_channel();
            tokio::spawn(
                UnboundedReceiverStream::new(receiver)
                    .map(Ok)
                    .forward(frame_sink),
            );
            UnboundedSenderSink::from(sender)
        };

        let fed = Federate::new(
            sink,
            federate_key,
            initial.neighbors,
            initial.clock_sync,
            start_time_sync.clone(),
        );

        federate_states.insert(federate_key, fed);

        frame_stream.map(move |msg| {
            msg.map(|inner| (federate_key, inner))
                .map_err(|err| Error::Other(err.into()))
        })
    });

    let stream_results = futures::stream::select_all(federate_streams);

    let rti_handle = tokio::spawn(
        Rti::new(
            federate_states,
            upstream_delay_map,
            start_time_sync.watcher(),
        )
        .run(stream_results),
    );

    let start_time = start_time_handle.await.map_err(|err| {
        tracing::error!("RTI failed to negotiate start time: {}", err);
        Error::Other(err.into())
    })?;

    // All federates have connected.
    tracing::debug!("All federates have connected to RTI.");

    Ok(RtiHandles {
        start_time,
        rti_handle,
        listener_handle,
    })
}

/// Listen for up to `number_of_federates` connections from federates, returning initial handshaking data.
async fn initialize_federates(
    number_of_federates: usize,
    federation_id: String,
    clock_sync_global_status: ClockSyncStat,
    listener: &TcpListener,
) -> Result<tinymap::TinySecondaryMap<FederateKey, Initial<TcpStream>>, Error> {
    let mut initial = tinymap::TinySecondaryMap::new();
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
                tracing::info!(?federate_id, "RTI accepted federate.");
                initial.insert(
                    federate_id,
                    Initial {
                        frame,
                        neighbors,
                        clock_sync,
                    },
                );
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
            tracing::debug!(%fed_ids, "RTI received FedIds.");

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
                    "RTI received an unexpected connection request. Federation is already running."
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

/// Build a transitive map of upstream delays for every federate pair.
///
/// `each_upstream` is an iterator over the upstream neighbors of each federate.
fn build_transitive_upstream_delay_map<'a, I>(
    each_upstream: I,
) -> tinymap::TinySecondaryMap<FederateKey, Vec<(FederateKey, Duration)>>
where
    I: Iterator<Item = (FederateKey, &'a Vec<(FederateKey, Duration)>)>,
{
    // Build weighted DAG of upstream edges from every federate.
    let edges = each_upstream.flat_map(|(key1, upstream)| {
        upstream
            .iter()
            .map(move |&(key2, d)| (key1, key2, d.as_secs_f64()))
    });

    let graph = petgraph::graphmap::DiGraphMap::from_edges(edges);

    graph
        .nodes()
        .map(|key| {
            let paths = petgraph::algo::bellman_ford(&graph, key.into()).unwrap();
            let distances = paths
                .distances
                .iter()
                .enumerate()
                .filter_map(|(ix, d)| {
                    d.is_finite()
                        .then(|| (FederateKey::from(ix), Duration::from_secs_f64(*d)))
                })
                .collect();
            (key, distances)
        })
        .collect()
}

#[test]
fn test_build_transitive_upstream_delay_map() {
    let feds = (0..=3).map(FederateKey::from).collect::<Vec<_>>();
    let upstream0 = vec![
        (feds[1], Duration::from_secs(1)),
        (feds[2], Duration::from_secs(1)),
        (feds[3], Duration::from_secs(3)),
    ];
    let upstream1 = vec![(feds[3], Duration::from_secs(1))];
    let upstream3 = vec![];
    let neighbors = vec![
        (feds[0], &upstream0),
        (feds[1], &upstream1),
        (feds[2], &upstream1),
        (feds[3], &upstream3),
    ];
    let map = build_transitive_upstream_delay_map(neighbors.iter().cloned());

    assert_eq!(
        map[feds[0]],
        vec![
            (feds[0], Duration::from_secs(0)),
            (feds[1], Duration::from_secs(1)),
            (feds[2], Duration::from_secs(1)),
            (feds[3], Duration::from_secs(2)),
        ]
    );
}
