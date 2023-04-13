use crate::{client, rti, FederateId, NeighborStructure};

#[test_log::test(tokio::test)]
async fn test1() {
    let mut server = rti::Rti::new(1, "fed1");
    let listener = server.create_listener(12345).await.unwrap();

    let server_handle = tokio::spawn(async move { server.start_server(listener).await });

    let neighbors = NeighborStructure {
        upstream: vec![],
        downstream: vec![],
    };

    let client = client::Federate::new(FederateId::from(0), "fed1", neighbors);
    client
        .connect_to_rti("127.0.0.1:12345".parse().unwrap())
        .await
        .unwrap();

    let x = server_handle.await.unwrap();
}
