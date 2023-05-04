use boomerang_core::{
    keys::PortKey,
    time::{Tag, Timestamp},
};

use crate::{
    client::{self, ClientError},
    rti, FederateKey, NeighborStructure, RejectReason,
};

#[test_log::test(tokio::test)]
async fn test1() {
    let listener = rti::create_listener(12345).await.unwrap();
    let server_handle = tokio::spawn(async move {
        rti::start_rti(listener, rti::Config::new("fed1").with_federates(1)).await
    });

    let (client1, handles1) = client::connect_to_rti(
        "127.0.0.1:12345".parse().unwrap(),
        client::Config::new(FederateKey::from(0), "fed1", NeighborStructure::default()),
    )
    .await
    .unwrap();

    let res2 = client::connect_to_rti(
        "127.0.0.1:12345".parse().unwrap(),
        client::Config::new(FederateKey::from(1), "fed1", NeighborStructure::default()),
    )
    .await;
    assert!(matches!(
        res2,
        Err(ClientError::Rejected(
            RejectReason::FederationIdDoesNotMatch
        ))
    ));

    let handles = server_handle.await.unwrap().unwrap();
}

#[test_log::test(tokio::test)]
async fn test2() {
    let listener = rti::create_listener(12345).await.unwrap();
    let server_handle = tokio::spawn(rti::start_rti(
        listener,
        rti::Config::new("fed1").with_federates(2),
    ));

    let res0 = client::connect_to_rti(
        "127.0.0.1:12345".parse().unwrap(),
        client::Config::new(FederateKey::from(0), "fed1", NeighborStructure::default()),
    );

    let res1 = client::connect_to_rti(
        "127.0.0.1:12345".parse().unwrap(),
        client::Config::new(FederateKey::from(1), "fed1", NeighborStructure::default()),
    );

    let (ret0, ret1) = futures::try_join!(res0, res1).unwrap();

    ret0.0
        .send_port_absent_to_federate(
            FederateKey::from(0),
            PortKey::from(0),
            Tag::now(Timestamp::ZERO),
        )
        .unwrap();

    let x = server_handle.await.unwrap().unwrap();
    x.rti_handle.await.unwrap().unwrap();
}
