use std::{
    net::{SocketAddr, TcpStream},
    time::Duration,
};

use server::*;

#[tokio::test]
async fn test_connections() {
    let address = (HOST_DEFAULT, PORT_DEFAULT);
    let server = server::Server::build(address).await.unwrap();
    let server_thread = tokio::spawn(server.run());
    tokio::time::sleep(Duration::from_millis(500)).await;
    let _: Vec<_> = (1..=100)
        .map(|_| TcpStream::connect(SocketAddr::from(address)).unwrap())
        .collect();
    tokio::time::sleep(Duration::from_secs(1)).await;
    if server_thread.is_finished() {
        server_thread.await.unwrap().unwrap();
    }
}
