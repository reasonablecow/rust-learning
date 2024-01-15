use core::time::Duration;
use std::{net::SocketAddr, path::PathBuf};

use tokio::net::TcpStream;

use client::*;

#[tokio::test]
async fn test_run() {
    let addr = SocketAddr::from((HOST_DEFAULT, PORT_DEFAULT));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("TCP listener creation should not fail.");
    let server_thread = tokio::spawn(async move {
        let mut v: Vec<TcpStream> = Vec::new();
        loop {
            let (socket, _) = listener.accept().await.expect("Accepting client failed!");
            v.push(socket);
        }
    });
    let client_thread = tokio::spawn(run(Config {
        img_dir: PathBuf::from("imgs"),
        file_dir: PathBuf::from("fls"),
        addr,
        save_png: true,
    }));
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(!server_thread.is_finished());
    assert!(!client_thread.is_finished());
}
