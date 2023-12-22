use std::time::Duration;

use cli_ser::{cli, ser, Data, Messageable};
use tokio::net::TcpStream;

use server::*;

async fn connect() -> TcpStream {
    let mut conn = TcpStream::connect(address_default())
        .await
        .expect("connecting to the server should succeed");
    let ser::Msg::Info(_) = ser::Msg::receive(&mut conn).await.unwrap() else {
        panic!()
    };
    (cli::Msg::Auth {
        username: "test".to_string(),
        password: "testtest".to_string(),
    })
    .send(&mut conn)
    .await
    .unwrap();
    let ser::Msg::Info(_) = ser::Msg::receive(&mut conn).await.unwrap() else {
        panic!()
    };
    conn
}

async fn send(socket: &mut TcpStream, s: &str) {
    cli::Msg::Data(Data::Text(s.to_string()))
        .send(socket)
        .await
        .expect("sending a message to the server should work");
}

async fn recv(socket: &mut TcpStream) -> String {
    let ser::Msg::DataFrom {
        data: Data::Text(s),
        ..
    } = ser::Msg::receive(socket).await.unwrap()
    else {
        panic!()
    };

    s.to_string()
}

#[tokio::test]
async fn test_5_clients_5_messages() {
    let server_thread = tokio::spawn(run(address_default()));
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Connection of client_1, client_2, client_3
    let mut client_1 = connect().await;
    let mut client_2 = connect().await;
    let mut client_3 = connect().await;

    // client_3 sends a message to client_1, client_2 (SEND AFTER CONNECTION)
    let msg_1 = "#1 from 3";
    send(&mut client_3, msg_1).await;
    // Wait for the broadcast.
    tokio::time::sleep(Duration::from_millis(100)).await;
    // Check if client_2 received it.
    assert_eq!(recv(&mut client_2).await, msg_1);
    // client_2 quits
    drop(client_2);

    // client_3 sends a message to client_1 (SEND AFTER QUIT)
    let msg_2 = "#2 from 3";
    send(&mut client_3, msg_2).await;
    // tokio::time::sleep(Duration::from_secs(1)).await; // Wait for client_1 to receive it

    // Connection of client_4
    let client_4 = connect().await;

    // client_1 sends a message to client_3, client_4 (MESSAGE FROM OTHER CLIENT)
    let msg_3 = "#3 from 1";
    send(&mut client_1, msg_3).await;
    // TODO: without this sleep, the client4 gets msg_4 and msg_5 before msg_3...
    tokio::time::sleep(Duration::from_millis(50)).await;

    // client_3 sends a message to client_1, client_4 (SEND AFTER OTHER SEND)
    let msg_4 = "#4 from 3";
    send(&mut client_3, msg_4).await;

    // client_3 sends a message to client_1, client_4 (SEND AFTER ITS OWN SEND)
    let msg_5 = "#5 from 3";
    send(&mut client_3, msg_5).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Connection of client_5
    let client_5 = connect().await;
    // Wait for all messages to arrive.
    tokio::time::sleep(Duration::from_secs(1)).await;

    for (mut stream, msgs) in [
        (client_1, vec![msg_1, msg_2, msg_4, msg_5]),
        (client_3, vec![msg_3]),
        (client_4, vec![msg_3, msg_4, msg_5]),
        (client_5, vec![]),
    ]
    .into_iter()
    {
        // TODO: change this to while let with timeout.
        let collected = async {
            let mut col: Vec<String> = Vec::new();
            for _ in 0..msgs.len() {
                col.push(recv(&mut stream).await);
            }
            col
        }
        .await;
        assert_eq!(collected, msgs);
    }
    assert!(!server_thread.is_finished());
}
