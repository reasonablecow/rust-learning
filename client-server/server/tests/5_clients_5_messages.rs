use cli_ser::Message::{self, Text};
use server::*;
use std::{iter::from_fn, net::TcpStream, time::Duration};

const CONN_ERR: &str = "connecting to the server should succeed";
const SEND_ERR: &str = "sending a message to the server should work";

fn to_text(s: &str) -> cli_ser::Message {
    Text(s.to_string())
}

fn from_text(t: Message) -> String {
    match t {
        Text(s) => s,
        _ => panic!("{:?}", t),
    }
}

#[tokio::test]
async fn test_5_clients_5_messages() {
    let address_default = address_default();
    let server_thread = tokio::spawn(run(address_default));
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Connection of client_1, client_2, client_3
    let mut client_1 = TcpStream::connect(address_default).expect(CONN_ERR);
    let mut client_2 = TcpStream::connect(address_default).expect(CONN_ERR);
    let mut client_3 = TcpStream::connect(address_default).expect(CONN_ERR);

    // client_3 sends a message to client_1, client_2 (SEND AFTER CONNECTION)
    let msg_1 = "#1 from 3";
    to_text(msg_1).send(&mut client_3).expect(SEND_ERR);
    // Wait for the broadcast.
    tokio::time::sleep(Duration::from_millis(100)).await;
    // Check if client_2 received it.
    assert_eq!(
        Message::receive(&mut client_2)
            .expect("Receiving message for client_2 failed")
            .map(from_text)
            .expect("Message received for client_2 should not be None"),
        msg_1
    );
    // client_2 quits
    drop(client_2);

    // client_3 sends a message to client_1 (SEND AFTER QUIT)
    let msg_2 = "#2 from 3";
    to_text(msg_2).send(&mut client_3).expect(SEND_ERR);
    tokio::time::sleep(Duration::from_secs(1)).await; // Wait for client_1 to receive it

    // Connection of client_4
    let client_4 = TcpStream::connect(address_default).expect(CONN_ERR);

    // client_1 sends a message to client_3, client_4 (MESSAGE FROM OTHER CLIENT)
    let msg_3 = "#3 from 1";
    to_text(msg_3).send(&mut client_1).expect(SEND_ERR);

    // client_3 sends a message to client_1, client_4 (SEND AFTER OTHER SEND)
    let msg_4 = "#4 from 3";
    to_text(msg_4).send(&mut client_3).expect(SEND_ERR);

    // client_3 sends a message to client_1, client_4 (SEND AFTER ITS OWN SEND)
    let msg_5 = "#5 from 3";
    to_text(msg_5).send(&mut client_3).expect(SEND_ERR);
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Connection of client_5
    let client_5 = TcpStream::connect(address_default).expect(CONN_ERR);
    // Wait for all messages to arrive.
    tokio::time::sleep(Duration::from_secs(1)).await;

    for (num, (mut stream, msgs)) in [
        (client_1, vec![msg_1, msg_2, msg_4, msg_5]),
        (client_3, vec![msg_3]),
        (client_4, vec![msg_3, msg_4, msg_5]),
        (client_5, vec![]),
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            from_fn(|| Message::receive(&mut stream).expect(&format!("{}", num)))
                .map(from_text)
                .collect::<Vec<_>>(),
            msgs
        )
    }
    assert!(!server_thread.is_finished());
}
