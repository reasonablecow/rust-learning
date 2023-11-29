use cli_ser::Message;
use server::*;
use std::{net::TcpStream, thread, time::Duration};

fn client(s: &str) -> Message {
    let msg = Message::Text(s.to_string());
    let mut stream =
        TcpStream::connect(ADDRESS_DEFAULT).expect("connecting to the server should succeed");
    thread::sleep(Duration::from_millis(100)); // wait for client connections
    msg.send(&mut stream)
        .expect("sending of bytes should succeed");
    thread::sleep(Duration::from_millis(100)); // wait for messages to be sent
    Message::receive(&mut stream)
        .expect("receiving of a message should succeed")
        .expect("message should not be None")
}

#[test]
fn test_2_clients_text_message() {
    let server_thread = thread::spawn(|| run(ADDRESS_DEFAULT));
    thread::sleep(Duration::from_millis(100));

    let s_1 = "hi from 1";
    let msg_1 = Message::Text(s_1.to_string());
    let conn_1 = thread::spawn(move || client(s_1));

    let s_2 = "hi from 2";
    let msg_2 = Message::Text(s_2.to_string());
    let conn_2 = thread::spawn(move || client(s_2));

    assert_eq!(conn_1.join().unwrap(), msg_2);
    assert_eq!(conn_2.join().unwrap(), msg_1);
    assert!(!server_thread.is_finished());
}
