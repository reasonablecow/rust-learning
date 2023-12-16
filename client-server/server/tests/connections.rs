use server::*;
use std::{net::TcpStream, thread, time::Duration};

#[tokio::test]
async fn test_connections() {
    let server_thread = tokio::spawn(run(address_default()));
    tokio::time::sleep(Duration::from_millis(500)).await;
    let _: Vec<_> = (1..=100)
        .map(|_| TcpStream::connect(address_default()).unwrap())
        .collect();
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(!server_thread.is_finished());
}
