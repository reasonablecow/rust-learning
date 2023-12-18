use std::time::Duration;

use cli_ser::Message;
use tokio::net::TcpStream;

use server::*;

async fn client(s: &str) -> Message {
    let mut stream = TcpStream::connect(address_default())
        .await
        .expect("connecting to the server should succeed");
    tokio::time::sleep(Duration::from_millis(100)).await; // wait for client connections
    Message::Text(s.to_string())
        .send(&mut stream)
        .await
        .expect("sending of bytes should succeed");
    tokio::time::sleep(Duration::from_millis(100)).await; // wait for messages to be sent
    Message::receive(&mut stream)
        .await
        .expect("receiving of a message should succeed")
}

fn msg_to_string(m: Message) -> String {
    match m {
        Message::Text(s) => s,
        other => panic!("{:?}", other),
    }
}

#[tokio::test]
async fn test_2_clients_text_message() {
    let server_thread = tokio::spawn(run(address_default()));
    tokio::time::sleep(Duration::from_millis(100)).await;

    let s_1 = "hi from 1";
    let conn_1 = tokio::spawn(client(s_1));

    let s_2 = "hi from 2";
    let conn_2 = tokio::spawn(client(s_2));

    assert_eq!(msg_to_string(conn_1.await.unwrap()), s_2.to_string());
    assert_eq!(msg_to_string(conn_2.await.unwrap()), s_1.to_string());
    assert!(!server_thread.is_finished());
}
