use server::*;
use std::{net::TcpStream, thread, time::Duration};

#[test]
fn test_connections() {
    let server_thread = thread::spawn(|| run(ADDRESS_DEFAULT));
    thread::sleep(Duration::from_millis(500));
    let _: Vec<_> = (1..=100)
        .map(|_| TcpStream::connect(ADDRESS_DEFAULT).unwrap())
        .collect();
    thread::sleep(Duration::from_secs(1));
    assert!(!server_thread.is_finished());
}
