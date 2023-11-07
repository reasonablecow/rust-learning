use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    thread,
    time::Duration,
};

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
pub enum Message {
    Text(String),
}

/// TODO
pub fn get_host_and_port() -> String {
    String::from("127.0.0.1:11111")
}

pub fn read_msg(stream: &mut TcpStream) -> Option<Message> {
    stream
        .set_nonblocking(true)
        .expect("Setting non-blocking stream to check for data to be read failed.");
    let mut len_bytes = [0u8; 4];
    match stream.read_exact(&mut len_bytes) {
        Ok(()) => {
            stream
                .set_nonblocking(false)
                .expect("Setting blocking stream to read the data.");
            let len = u32::from_be_bytes(len_bytes) as usize;
            println!("len! {:?}", len);
            let mut msg_buf = vec![0u8; len];
            stream
                .read_exact(&mut msg_buf)
                .expect("Reading the whole message should be ok.");
            let msg: Message = bincode::deserialize(&msg_buf[..])
                .expect("Deserialization of the read message should be ok.");
            println!("msg! {:?}", msg);
            Some(msg)
        }
        Err(e) => {
            //println!("{:?}", e),
            // Check only the errors caused by nonblocking
            None
        }
    }
}

pub fn send_msg(stream: &mut TcpStream, msg: Message) {
    let bytes = bincode::serialize(&msg).expect("Message serialization should work fine.");
    let len = bytes.len() as u32;

    stream
        .write(&len.to_be_bytes())
        .expect("Writing to stream should work flawlessly.");
    stream
        .write_all(&bytes)
        .expect("Writing the serialized message should be ok.");
    stream.flush().expect("flushing the stream should be ok");
}

pub fn simulate_connections() {
    let address = get_host_and_port();

    let connection_simulator = thread::spawn(move || {
        let mut streams = Vec::new();
        for sth in ["one", "two", "three", "four", "five"] {
            let mut stream = TcpStream::connect(address.clone())
                .expect("TCP stream connection from another thread should be possible.");
            let msg = Message::Text(sth.to_string());
            send_msg(&mut stream, msg);
            streams.push(stream);
        }
        streams
    });
    let streams = connection_simulator
        .join()
        .expect("the streams should be returned.");
    println!("{:?}", streams);
    thread::sleep(Duration::from_secs(3));
}

pub fn add(left: usize, right: usize) -> usize {
    left + right
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}
