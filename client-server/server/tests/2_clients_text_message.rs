use std::{net::SocketAddr, time::Duration};

use cli_ser::{
    cli::{self, Auth::LogIn, Auth::SignUp, Credentials, Msg::Auth},
    ser, Data, Messageable,
};
use tokio::net::TcpStream;

use server::*;

async fn client(s: &str, creds: Credentials) -> Data {
    let mut stream = TcpStream::connect(SocketAddr::from((HOST_DEFAULT, PORT_DEFAULT)))
        .await
        .expect("Connecting to the server failed!");
    Auth(LogIn(creds))
        .send(&mut stream)
        .await
        .expect("sending Auth failed");

    // wait for the other client to connect
    tokio::time::sleep(Duration::from_secs(1)).await;

    cli::Msg::ToAll(Data::Text(s.to_string()))
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
    let address = (HOST_DEFAULT, PORT_DEFAULT);
    let server = server::Server::build(address).await.unwrap();
    let server_thread = tokio::spawn(server.run());
    tokio::time::sleep(Duration::from_millis(100)).await;

    let creds = Credentials {
        user: "test".to_string().into(),
        password: "test_pass".to_string(),
    };
    {
        let mut stream = TcpStream::connect(SocketAddr::from(address)).await.unwrap();
        Auth(SignUp(creds.clone())).send(&mut stream).await.unwrap();
        match ser::Msg::receive(&mut stream).await.unwrap() {
            ser::Msg::Authenticated | ser::Msg::Error(ser::Error::UsernameTaken) => {}
            other => panic!("{other:?}"),
        }
    }
    let s_1 = "hi from 1";
    let conn_1 = tokio::spawn(client(s_1, creds.clone()));

    let s_2 = "hi from 2";
    let conn_2 = tokio::spawn(client(s_2, creds.clone()));

    assert_eq!(data_to_string(conn_1.await.unwrap()), s_2.to_string());
    assert_eq!(data_to_string(conn_2.await.unwrap()), s_1.to_string());
    if server_thread.is_finished() {
        server_thread.await.unwrap().unwrap();
    }
}
