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
    io::ErrorKind::BrokenPipe,
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{mpsc, Arc, Mutex},
    thread,
};

use chrono::{offset::Utc, SecondsFormat};
use clap::Parser;
use tracing::info;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{
    filter::LevelFilter, prelude::__tracing_subscriber_SubscriberExt, util::SubscriberInitExt,
    Layer,
};

use crate::Task::*;
use cli_ser::{
    send_bytes,
    Error::{SendByteCnt, SendBytes},
    Message,
};

const MSCP_ERROR: &str = "Sending message over the mpsc channel should always work.";
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
            let stream = incoming.expect("Incoming Stream should be Ok");
            info!("incoming {:?}", stream);
            task_taker_clone.send(NewStream(stream)).expect(MSCP_ERROR);
        }
    });

    let workers = ThreadPool::new(6);
    let mut streams: HashMap<SocketAddr, TcpStream> = HashMap::new();
    for task in task_giver {
        match task {
            NewStream(stream) => {
                let addr = stream
                    .peer_addr()
                    .expect("Every stream should have accessible address.");
                streams.insert(addr, stream);
                task_taker.send(Check(addr)).expect(MSCP_ERROR);
            }
            Check(addr) => {
                if let Some(stream) = streams.get(&addr) {
                    let task_taker_clone = task_taker.clone();
                    let mut stream_clone = stream.try_clone().expect("Stream should be cloneable.");

                    workers.execute(move || {
                        if let Some(msg) = Message::receive(&mut stream_clone)
                            .expect("message receiving should not fail")
                        {
                            task_taker_clone
                                .send(Broadcast(addr, msg))
                                .expect(MSCP_ERROR);
                        }
                        task_taker_clone.send(Check(addr)).expect(MSCP_ERROR);
                    });
                } // The stream was removed from streams after the Check creation.
            }
            Broadcast(addr_from, msg) => {
                info!("broadcasting message from {:?}", addr_from);
                let bytes = msg
                    .serialize()
                    .expect("message serialization should not fail");

                for (&addr_to, stream) in streams.iter() {
                    if addr_from != addr_to {
                        let task_taker_clone = task_taker.clone();
                        let mut stream_clone =
                            stream.try_clone().expect("Stream should be cloneable.");
                        let bytes_clone = bytes.clone();

                        workers.execute(move || {
                            match send_bytes(&mut stream_clone, &bytes_clone) {
                                Ok(()) => {}
                                Err(SendByteCnt(e)) | Err(SendBytes(e))
                                    if e.kind() == BrokenPipe =>
                                {
                                    task_taker_clone
                                        .send(StreamClose(addr_to))
                                        .expect(MSCP_ERROR);
                                }
                                other => panic!("{:?}", other),
                            }
                        });
                    }
                }
            }
            StreamClose(addr) => {
                info!("disconnected {}", addr);
                streams
                    .remove(&addr)
                    .expect("Stream was present and should have been so until now.");
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
        let (channel, job_recv) = mpsc::channel();
        let job_recv = Arc::new(Mutex::new(job_recv));

        let _threads = (0..thread_cnt)
            .map(|_| {
                let job_recv_clone = Arc::clone(&job_recv);
                thread::spawn(move || loop {
                    let job: Job = job_recv_clone.lock().unwrap().recv().unwrap();
                    job();
                })
            })
            .collect::<Vec<_>>();
        ThreadPool { channel, _threads }
    }

    pub fn execute<F>(&self, f: F)
    where
        F: FnOnce() + Send + 'static,
    {
        self.channel.send(Box::new(f)).unwrap();
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
