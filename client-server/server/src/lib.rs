//! # Server Executable
//!
//! Listens at a specified address and broadcasts every received message to all other connected clients.
//! See:
//! ```sh
//! cargo run -- --help
//! ```
//!
//! TODO: Messages received from one client should be broadcasted in the same order as received.
use std::{
    collections::HashMap,
    fs,
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{mpsc, Arc, Mutex},
    thread,
};

use chrono::{offset::Utc, SecondsFormat};
use clap::Parser;
use tracing::{error, info, warn};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{
    filter::LevelFilter, prelude::__tracing_subscriber_SubscriberExt, util::SubscriberInitExt,
    Layer,
};

use crate::Task::*;
use cli_ser::{send_bytes, Error::DisconnectedStream, Message};

pub const ADDRESS_DEFAULT: &str = "127.0.0.1:11111";

/// Server executable, listens at specified address and broadcasts messages to all connected clients.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// Server host
    #[arg(long, default_value_t = String::from("127.0.0.1"))]
    host: String,

    /// Server port
    #[arg(short, long, default_value_t = 11111)]
    port: u32,
}
impl Args {
    pub fn parse_to_address() -> String {
        let args = Self::parse();
        format!("{}:{}", args.host, args.port)
    }
}

/// Tasks to be initially queued at the server and addressed later.
#[derive(Debug)]
enum Task {
    NewStream(TcpStream),
    Check(SocketAddr),
    SendErrMsg(SocketAddr, String),
    Broadcast(SocketAddr, Message),
    StreamClose(SocketAddr),
}

/// The server's main function consists of a "listening" thread and the server's main loop.
///
/// The server is bound to a specified address (host and port).
/// A separate "listening" thread is dedicated to capturing new clients.
/// In the main loop, the server processes tasks one at a time from its queue.
/// Small tasks are resolved immediately, larger once are delegated to a thread pool.
///
/// TODO: Make thread pool size configurable by the caller.
pub fn run(address: &str) {
    let (task_taker, task_giver) = mpsc::channel();

    let listener = TcpListener::bind(address).expect("TCP listener creation should not fail.");
    info!("Server is listening at {:?}", address);

    let task_taker_clone = task_taker.clone();
    let _stream_receiver = thread::spawn(move || {
        for incoming in listener.incoming() {
            match incoming {
                Ok(stream) => {
                    info!("incoming {:?}", stream);
                    task_taker_clone
                        .send(NewStream(stream))
                        .unwrap_or_else(|e| error!("sending NewStream to tasks failed: {:?}", e))
                }
                Err(e) => error!("incoming stream error: {:?}", e),
            }
        }
    });

    let workers = ThreadPool::new(6);
    let mut streams: HashMap<SocketAddr, TcpStream> = HashMap::new();
    for task in task_giver {
        match task {
            NewStream(stream) => match stream.peer_addr() {
                Ok(addr) => {
                    streams.insert(addr, stream);
                    task_taker.send(Check(addr)).unwrap_or_else(|e| {
                        error!("sending Check(\"{}\") to tasks failed, error: {}", addr, e)
                    })
                }
                Err(e) => error!(
                    "reading address of new stream {:?} failed, error {}",
                    stream, e
                ),
            },
            Check(addr) => {
                if let Some(stream) = streams.get(&addr) {
                    let task_taker_clone = task_taker.clone();
                    match stream.try_clone() {
                        Ok(mut stream_clone) => {
                            workers.execute(move || {
                                let check_again = || task_taker_clone.send(Check(addr)).unwrap_or_else(|e| error!("sending Check({addr}) to tasks failed, error {e}"));
                                match Message::receive(&mut stream_clone) {
                                    Ok(Some(msg)) => {
                                        task_taker_clone
                                            .send(Broadcast(addr, msg))
                                            .unwrap_or_else(|e| error!("sending Broadcast from \"{addr}\" to tasks failed, error {e:?}"));
                                        check_again();
                                    }
                                    Ok(None) => check_again(),
                                    Err(cli_ser::Error::DisconnectedStream(_)) => {
                                        info!("disconnected {}", addr);
                                        task_taker_clone.send(StreamClose(addr)).unwrap_or_else(|e| error!("sending StreamClose({addr}) to tasks failed, error {e}"));
                                    }
                                    Err(e) => {
                                        error!("receiving message from stream {addr} failed, error {e}");
                                        check_again();
                                    }
                                }
                            }).unwrap_or_else(|e| error!("sending a job to the thread pool failed, error {e}"))
                        }
                        Err(e) => error!("cloning stream to check incoming message failed {}", e),
                    }
                } else {
                    warn!("stream for address {} wasn't found in order to check for incoming messages", addr)
                } // The stream was removed from streams after the Check creation.
            }
            SendErrMsg(addr, body) => {
                let fail_msg = format!(
                    "sending error message \"{:?}\", to \"{:?}\" failed",
                    body, addr
                );
                let fail_msg_clone = fail_msg.clone();
                match streams.get(&addr).map(|s| s.try_clone()) {
                    Some(Ok(mut stream)) => workers.execute(move || {
                        Message::Error(body)
                            .send(&mut stream)
                            .unwrap_or_else(|e| error!("{fail_msg_clone}, error: {e:?}"));
                    }).unwrap_or_else(|e| error!("{fail_msg}, because sending a job to the thread pool failed, error {e}")),
                    Some(Err(e)) => error!("{fail_msg}, because cloning stream for address {addr:?} failed, error {e:?}"),
                    None => error!(
                        "{}, because stream for address {:?} was not found",
                        fail_msg, addr
                    ),
                }
            }
            Broadcast(addr_from, msg) => {
                info!("broadcasting message from {:?}", addr_from);
                match msg.serialize() {
                    Ok(bytes) => {
                        for (&addr_to, stream) in streams.iter() {
                            if addr_from != addr_to {
                                match stream.try_clone() {
                                    Ok(mut stream_clone) => {
                                        let task_taker_clone = task_taker.clone();
                                        let bytes_clone = bytes.clone();

                                        workers.execute(move || {
                                        match send_bytes(&mut stream_clone, &bytes_clone) {
                                            Ok(()) => {}
                                            Err(DisconnectedStream(_)) => {}
                                            other => {
                                                let body = format!("broadcasting message to client \"{:?}\" failed, error: {:?}", addr_to, other);
                                                task_taker_clone.send(Task::SendErrMsg(addr_from, body.clone()))
                                                    .unwrap_or_else(|e| error!("sending SendErrMsg({addr_from}, {body}) to tasks failed, error {e}"))
                                                }
                                            }
                                        }).unwrap_or_else(|e| error!("sending a broadcast job (from {addr_from} to {addr_to}) to the thread pool failed, error {e}"));
                                    }
                                    Err(e) => error!("cloning stream of {addr_to} (in order to broadcast message) failed, error {e}"),
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error!("serialization of a message (from {addr_from}) failed, error {e}")
                    }
                }
            }
            StreamClose(addr) => {
                streams.remove(&addr).expect(
                    "removing stream should never fail, contact the implementer! (address {addr})",
                );
            }
        }
    }
}

type Job = Box<dyn FnOnce() + Send + 'static>;

/// Structure for executing jobs in parallel on fixed number of threads.
///
/// Inspired by: <https://doc.rust-lang.org/book/ch20-02-multithreaded.html>
struct ThreadPool {
    channel: mpsc::Sender<Job>,
    _threads: Vec<thread::JoinHandle<()>>,
}
impl ThreadPool {
    fn new(thread_cnt: usize) -> ThreadPool {
        let (channel, job_recv) = mpsc::channel::<Job>();
        let job_recv = Arc::new(Mutex::new(job_recv));

        let _threads = (0..thread_cnt)
            .map(|_| {
                let job_recv_clone = Arc::clone(&job_recv);
                thread::spawn(move || loop {
                    let job_result = job_recv_clone
                        .lock()
                        .expect("acquiring a mutex to receive a job failed")
                        .recv();
                    match job_result {
                        Ok(job) => job(),
                        Err(e) => error!("receiving a job from job queue failed, error {e}"),
                    }
                })
            })
            .collect::<Vec<_>>();
        ThreadPool { channel, _threads }
    }

    pub fn execute<F>(&self, f: F) -> std::result::Result<(), Box<dyn std::error::Error>>
    where
        F: FnOnce() + Send + 'static,
    {
        self.channel.send(Box::new(f))?;
        Ok(())
    }
}

/// Subscribes to tracing (and logging), outputs to stdout and a log file.
///
/// Returns WorkerGuard which must be kept for the intended time of log capturing.
pub fn init_logging_stdout_and_file() -> WorkerGuard {
    let term_layer = tracing_subscriber::fmt::layer().with_filter(LevelFilter::INFO);

    let file = fs::File::create(format!(
        "{}.log",
        Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
    ))
    .expect("Log file creation should be possible, please check your permissions.");
    let (non_blocking, guard) = tracing_appender::non_blocking(file);
    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(non_blocking)
        .with_filter(LevelFilter::TRACE);

    tracing_subscriber::registry()
        .with(term_layer)
        .with(file_layer)
        .init(); // sets itself as global default subscriber

    guard
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_run_1_sec() {
        let server_thread = thread::spawn(|| run(ADDRESS_DEFAULT));
        thread::sleep(Duration::from_secs(1));
        assert!(!server_thread.is_finished());
    }
}
