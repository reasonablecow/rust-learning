use std::{
    collections::HashMap,
    io::Write,
    net::{SocketAddr, TcpListener, TcpStream},
    sync::mpsc,
    thread,
};

use crate::Task::*;
use cli_ser::{get_host_and_port, read_msg, Message};

const MSCP_ERROR: &str = "Sending message over the mpsc channel should always work.";

#[derive(Debug)]
enum Task {
    NewStream(TcpStream),
    Check(SocketAddr),
    Broadcast(SocketAddr, Message),
}

fn main() {
    println!("i am the server!");
    let address = get_host_and_port();

    let (sender, receiver) = mpsc::channel();

    let listener = TcpListener::bind(address).expect("TCP listener creation should not fail.");

    let sender_clone = sender.clone();
    let _stream_receiver = thread::spawn(move || {
        for incoming in listener.incoming() {
            let /*mut*/ stream = incoming.expect("Incoming Stream should be Ok");
            println!("incoming {:?}", stream);
            sender_clone.send(NewStream(stream)).expect(MSCP_ERROR);
        }
    });

    let mut streams: HashMap<SocketAddr, TcpStream> = HashMap::new();
    for task in receiver {
        match task {
            NewStream(stream) => {
                let addr = stream
                    .peer_addr()
                    .expect("Every stream should have accessible address.");
                streams.insert(addr /*.clone()*/, stream);
                println!("streams {:?}", streams);
                sender.send(Check(addr)).expect(MSCP_ERROR);
            }
            Check(addr) => {
                let /*mut*/ stream = streams
                    .get(&addr)
                    .expect("All addresses are added before checked.");

                let sender_clone = sender.clone();
                let mut stream_clone = stream.try_clone().expect("Stream should be cloneable.");

                let _check_thread = thread::spawn(move || {
                    if let Some(msg) = read_msg(&mut stream_clone) {
                        sender_clone.send(Broadcast(addr, msg)).expect(MSCP_ERROR);
                    }
                    sender_clone.send(Check(addr)).expect(MSCP_ERROR);
                });
            }
            Broadcast(addr, msg) => {
                println!("{:?} {:?}", addr, msg);
                for (other_addr, mut stream) in &streams {
                    if addr != *other_addr {
                        println!("differ {:?} {:?}", addr, other_addr);

                        let bytes = bincode::serialize(&msg)
                            .expect("Message serialization should work fine.");
                        let len = bytes.len() as u32;

                        stream
                            .write_all(&len.to_be_bytes())
                            .expect("Writing to stream should work flawlessly.");
                        stream
                            .write_all(&bytes)
                            .expect("Writing the serialized message should be ok.");
                        stream.flush().expect("flushing the stream should be ok");
                    }
                }
            }
        }
    }
}
