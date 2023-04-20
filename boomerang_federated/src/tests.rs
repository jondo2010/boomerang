use crate::{client, rti, FederateKey, NeighborStructure};

#[test_log::test(tokio::test)]
async fn test1() {
    let mut server = rti::Rti::new(1, "fed1");
    let listener = server.create_listener(12345).await.unwrap();
    let server_handle = tokio::spawn(async move { server.start_server(listener).await });

    let neighbors = NeighborStructure {
        upstream: vec![],
        downstream: vec![],
    };

    let config = client::Config::new(FederateKey::from(0), "fed1", neighbors);
    let (client, handles) = client::connect_to_rti("127.0.0.1:12345".parse().unwrap(), config)
        .await
        .unwrap();

    let handles = server_handle.await.unwrap();
}
