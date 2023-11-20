use std::{
    collections::HashMap,
    io::ErrorKind::BrokenPipe,
    net::{SocketAddr, TcpListener, TcpStream},
    sync::mpsc,
    thread,
};

use clap::Parser;

use crate::Task::*;
use cli_ser::{read_msg, send_bytes, serialize_msg, Message};

const MSCP_ERROR: &str = "Sending message over the mpsc channel should always work.";

/// Server executable, listens at specified address and broadcasts messages to all connected clients.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Server host
    #[arg(long, default_value_t = String::from("127.0.0.1"))]
    host: String,

    /// Server port
    #[arg(short, long, default_value_t = 11111)]
    port: u32,
}

/// Server tasks which are queued and addressed.
#[derive(Debug)]
enum Task {
    NewStream(TcpStream),
    Check(SocketAddr),
    Broadcast(SocketAddr, Message),
    StreamClose(SocketAddr),
}

/// Server's main function consisting of a "welcoming" thread and server's main loop.
///
/// The server listens at specified address (host and port).
/// One separate "welcoming" thread is dedicated to capture new clients.
/// In the main loop the server takes one task at a time from a queue.
/// Small tasks solves by itself and for more complicated once spawns a new thread.
fn main() {
    let args = Args::parse();

    let (sender, receiver) = mpsc::channel();

    let address = format!("{}:{}", args.host, args.port);
    let listener = TcpListener::bind(&address).expect("TCP listener creation should not fail.");
    println!("Server is listening at {:?}", address);

    let sender_clone = sender.clone();
    let _stream_receiver = thread::spawn(move || {
        for incoming in listener.incoming() {
            let stream = incoming.expect("Incoming Stream should be Ok");
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
                streams.insert(addr, stream);
                sender.send(Check(addr)).expect(MSCP_ERROR);
            }
            Check(addr) => {
                if let Some(stream) = streams.get(&addr) {
                    let sender_clone = sender.clone();
                    let mut stream_clone = stream.try_clone().expect("Stream should be cloneable.");

                    let _check_thread = thread::spawn(move || {
                        if let Some(msg) = read_msg(&mut stream_clone) {
                            sender_clone.send(Broadcast(addr, msg)).expect(MSCP_ERROR);
                        }
                        sender_clone.send(Check(addr)).expect(MSCP_ERROR);
                    });
                } // The stream was removed from streams after the Check creation.
            }
            Broadcast(addr_from, msg) => {
                println!("broadcasting message from {:?}", addr_from);
                let bytes = serialize_msg(&msg);

                for (&addr_to, stream) in &streams {
                    if addr_from != addr_to {
                        let sender_clone = sender.clone();
                        let mut stream_clone =
                            stream.try_clone().expect("Stream should be cloneable.");
                        let bytes_clone = bytes.clone();

                        let _sender_thread = thread::spawn(move || {
                            match send_bytes(&mut stream_clone, &bytes_clone) {
                                Ok(()) => {}
                                Err(e) if e.kind() == BrokenPipe => {
                                    sender_clone.send(StreamClose(addr_to)).expect(MSCP_ERROR);
                                }
                                other => panic!("{:?}", other),
                            }
                        });
                    }
                }
            }
            StreamClose(addr) => {
                println!("disconnected {}", addr);
                streams
                    .remove(&addr)
                    .expect("Stream was present and should have been so until now.");
            }
        }
    }
}
