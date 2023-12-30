use std::time::Duration;

use cli_ser::{
    cli::{self, Auth::LogIn, Auth::SignUp, Msg::Auth, User},
    ser, Data, Messageable,
};
use tokio::net::TcpStream;

use server::*;

async fn client(s: &str, user: User) -> Data {
    let mut stream = TcpStream::connect(address_default())
        .await
        .expect("Connecting to the server failed!");
    Auth(LogIn(user))
        .send(&mut stream)
        .await
        .expect("sending Auth failed");

    // wait for the other client to connect
    tokio::time::sleep(Duration::from_secs(1)).await;

    cli::Msg::Data(Data::Text(s.to_string()))
        .send(&mut stream)
        .await
        .expect("sending of bytes should succeed");
    match ser::Msg::receive(&mut stream).await.unwrap() {
        ser::Msg::Authenticated => {}
        o => panic!("{o:?}"),
    };
    match ser::Msg::receive(&mut stream).await.unwrap() {
        ser::Msg::DataFrom { data, .. } => data,
        o => panic!("{o:?}"),
    }
}

fn data_to_string(m: Data) -> String {
    match m {
        Data::Text(s) => s,
        other => panic!("{:?}", other),
    }
}

#[tokio::test]
async fn test_2_clients_text_message() {
    let server = server::Server::build(address_default()).await.unwrap();
    let server_thread = tokio::spawn(server.run());
    tokio::time::sleep(Duration::from_millis(100)).await;

    let user = User {
        username: "test".to_string(),
        password: "test_pass".to_string(),
    };
    {
        let mut stream = TcpStream::connect(address_default()).await.unwrap();
        Auth(SignUp(user.clone())).send(&mut stream).await.unwrap();
        match ser::Msg::receive(&mut stream).await.unwrap() {
            ser::Msg::Authenticated | ser::Msg::Error(ser::Error::UsernameTaken) => {}
            other => panic!("{other:?}"),
        }
    }
    let s_1 = "hi from 1";
    let conn_1 = tokio::spawn(client(s_1, user.clone()));

    let s_2 = "hi from 2";
    let conn_2 = tokio::spawn(client(s_2, user.clone()));

    assert_eq!(data_to_string(conn_1.await.unwrap()), s_2.to_string());
    assert_eq!(data_to_string(conn_2.await.unwrap()), s_1.to_string());
    if server_thread.is_finished() {
        server_thread.await.unwrap().unwrap();
    }
}
