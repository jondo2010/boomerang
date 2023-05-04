//! `start_time_sync` implements the start time negotiation between federates and the RTI.
//!
//! A [`tokio::sync::mpsc`] channel is used to send a suggested start time from each federate to the
//! RTI. When the RTI has received a start time from each federate, it will select the maximum start
//! time and send it back to each federate using a [`tokio::sync::watch`] channel.
//!
//! The federates will then wait for the RTI to send them a message indicating that all federates
//! have received their start time. At this point, the federates will start executing.

use futures::StreamExt;
use tokio::sync::{mpsc, watch};
use tokio_stream::wrappers::ReceiverStream;

use crate::Timestamp;

/// The federate side of the start time negotiation.
#[derive(Debug, Clone)]
pub struct StartSync {
    /// Channel used to send start times from federates to the RTI.
    start_time_proposals: mpsc::Sender<Timestamp>,
    /// Channel used to receive the start time from the RTI.
    start_time: watch::Receiver<Timestamp>,
}

impl StartSync {
    /// Propose a start time to the RTI from a federate, and wait for the RTI to respond with the
    /// negotiated start time.
    pub async fn propose_start_time(
        &mut self,
        proposal: Timestamp,
    ) -> Result<Timestamp, watch::error::RecvError> {
        // Send a start time proposal to the RTI
        self.start_time_proposals.send(proposal).await.unwrap();

        // Receive the start time from the RTI.
        self.start_time
            .changed()
            .await
            .map(|_| self.start_time.borrow().clone())
    }

    /// Return a clone of the `start_time` channel's receiver.
    pub fn watcher(&self) -> watch::Receiver<Timestamp> {
        self.start_time.clone()
    }
}

/// `Syncronizer` receives start time proposals from federates and selects the maximum start time.
pub struct Synchronizer {
    /// Number of federates in the federation.
    num_federates: usize,
    /// Channel used to receive start times from federates.
    start_time_proposals: tokio::sync::mpsc::Receiver<Timestamp>,
    /// Channel used to send the start time to the federates.
    start_time: tokio::sync::watch::Sender<Timestamp>,
}

impl Synchronizer {
    /// Negotiate the start time from the Rti side.
    pub async fn negotiate_start_time(self) -> Timestamp {
        tracing::debug!(
            "Waiting for start time proposals from {} federates..",
            self.num_federates
        );

        // Receive `num_federates` start time proposals.
        let proposals = ReceiverStream::new(self.start_time_proposals)
            .inspect(|proposal| tracing::debug!("Received start time proposal: {proposal:?}"))
            .take(self.num_federates)
            .collect::<Vec<_>>()
            .await;

        // Select the maximum start time.
        let max_start_time = proposals
            .into_iter()
            .max()
            .expect("No start time proposals received");

        // Send the start time to the federates.
        self.start_time
            .send(max_start_time)
            .expect("Failed to send start time");

        tracing::debug!("Negotiated start time: {:?}", max_start_time);
        max_start_time
    }
}

/// Create a new `StartSync` and `Synchronizer` pair.
pub fn create(num_federates: usize) -> (StartSync, Synchronizer) {
    let (proposals_tx, proposals_rx) = tokio::sync::mpsc::channel(1);
    let (start_time_tx, start_time_rx) = tokio::sync::watch::channel(Timestamp::ZERO);

    let federate = StartSync {
        start_time_proposals: proposals_tx,
        start_time: start_time_rx,
    };

    let synchronizer = Synchronizer {
        num_federates,
        start_time_proposals: proposals_rx,
        start_time: start_time_tx,
    };

    (federate, synchronizer)
}

#[tokio::test]
async fn test_start_time_sync() {
    let (federate, synchronizer) = create(2);

    let sync_handle = tokio::spawn(synchronizer.negotiate_start_time());

    let mut fed1 = federate.clone();
    let federate_handle1 =
        tokio::spawn(async move { fed1.propose_start_time(Timestamp::now()).await.unwrap() });

    let mut fed2 = federate.clone();
    let federate_handle2 =
        tokio::spawn(async move { fed2.propose_start_time(Timestamp::now()).await.unwrap() });

    let rti_start_time = sync_handle.await.unwrap();
    let federate1_start_time = federate_handle1.await.unwrap();
    let federate2_start_time = federate_handle2.await.unwrap();

    assert_eq!(rti_start_time, federate1_start_time);
    assert_eq!(rti_start_time, federate2_start_time);
}
