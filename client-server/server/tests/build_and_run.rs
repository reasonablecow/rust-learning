use std::time::Duration;

use server::*;

#[tokio::test]
async fn test_run_1_sec() {
    let server = Server::build((HOST_DEFAULT, PORT_DEFAULT)).await.unwrap();
    let server_thread = tokio::spawn(server.run());
    std::thread::sleep(Duration::from_secs(1));
    assert!(!server_thread.is_finished());
}
