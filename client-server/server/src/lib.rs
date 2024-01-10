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

use anyhow::Context;
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
    SendErr(SocketAddr, cli_ser::ServerErr),
    Broadcast(SocketAddr, Message),
    StreamClose(SocketAddr),
}

type Streams = HashMap<SocketAddr, TcpStream>;

/// The server's main function consists of a "listening" thread and the server's main loop.
///
/// The server is bound to a specified address (host and port).
/// A separate "listening" thread is dedicated to capturing new clients.
/// In the main loop, the server processes tasks one at a time from its queue.
/// Small tasks are resolved immediately, larger once are delegated to a thread pool.
///
/// TODO: Make thread pool size configurable by the caller.
pub fn run(address: &str) -> anyhow::Result<()> {
    let (task_taker, task_giver) = mpsc::channel();

    let listener =
        TcpListener::bind(address).with_context(|| "TCP listener creation should not fail.")?;
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
    let mut streams: Streams = HashMap::new();
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
                check(addr, &streams, &workers, task_taker.clone());
            }
            SendErr(addr, err) => {
                let fail_msg = format!("sending error message \"{err:?}\", to \"{addr:?}\" failed");
                let fail_msg_clone = fail_msg.clone();

                match streams.get(&addr).map(|s| s.try_clone()) {
                    Some(Ok(mut stream)) => workers.execute(move || {
                        Message::ServerErr(err).send(&mut stream).unwrap_or_else(|e| error!("{fail_msg_clone}, error: {e:?}"));
                    }).unwrap_or_else(|e| error!("{fail_msg}, because sending a job to the thread pool failed, error {e}")),
                    Some(Err(e)) => error!("{fail_msg}, because cloning stream for address {addr:?} failed, error {e:?}"),
                    None => error!("{fail_msg}, because stream for address {addr:?} was not found")
                }
            }
            Broadcast(addr_from, msg) => {
                info!("broadcasting message from {:?}", addr_from);
                broadcast(msg, addr_from, &streams, &workers, &task_taker)
                    .with_context(|| "broadcasting message from {addr_from} failed")
                    .unwrap_or_else(|e| error!("{}", e))
            }
            StreamClose(addr) => {
                streams.remove(&addr).with_context(|| {
                    "removing stream should never fail, contact the implementer! (address {addr})"
                })?;
            }
        }
    }
    Ok(())
}

/// Checks address for incoming messages, queues broadcast and another check.
///
/// TODO: Send ServerErr message when receiving fails.
fn check(addr: SocketAddr, streams: &Streams, workers: &ThreadPool, tasks: mpsc::Sender<Task>) {
    let Some(stream) = streams.get(&addr) else {
        // The stream was probably removed after the Check creation.
        warn!("stream for address {addr} wasn't found in order to check for incoming messages");
        return;
    };
    let mut stream_clone = match stream.try_clone() {
        Ok(sc) => sc,
        Err(e) => return error!("cloning stream to check incoming message failed {e}"),
    };
    workers
        .execute(move || {
            let check_again = || {
                tasks
                    .send(Check(addr))
                    .unwrap_or_else(|e| error!("sending Check({addr}) to tasks failed, error {e}"))
            };
            match Message::receive(&mut stream_clone) {
                Ok(Some(msg)) => {
                    tasks.send(Broadcast(addr, msg)).unwrap_or_else(|e| {
                        error!("sending Broadcast from \"{addr}\" to tasks failed, error {e:?}")
                    });
                    check_again();
                }
                Ok(None) => check_again(),
                Err(cli_ser::Error::DisconnectedStream(_)) => {
                    info!("disconnected {}", addr);
                    tasks.send(StreamClose(addr)).unwrap_or_else(|e| {
                        error!("sending StreamClose({addr}) to tasks failed, error {e}")
                    });
                }
                Err(e) => {
                    error!("receiving message from stream {addr} failed, error {e}");
                    check_again();
                }
            }
        })
        .unwrap_or_else(|e| error!("sending a job to the thread pool failed, error {e}"))
}

/// Sends message to all streams except the `addr_from` using workers ThreadPool
fn broadcast(
    msg: Message,
    addr_from: SocketAddr,
    streams: &Streams,
    workers: &ThreadPool,
    tasks: &mpsc::Sender<Task>,
) -> anyhow::Result<()> {
    let bytes = msg
        .serialize()
        .with_context(|| "message serialization failed")?;
    for (&addr_to, stream) in streams.iter() {
        if addr_from != addr_to {
            let mut stream_clone = stream
                .try_clone()
                .with_context(|| "cloning stream failed")?;
            let bytes_clone = bytes.clone();
            let tasks_clone = tasks.clone();

            let err_msg = |addr| format!("sending message to client \"{addr:?}\" failed");
            workers.execute(move || match send_bytes(&mut stream_clone, &bytes_clone) {
                Ok(()) => {}
                Err(DisconnectedStream(_)) => {}
                other => {
                    let err = cli_ser::ServerErr::Sending(format!("{}, because {other:?}", err_msg(addr_to)));
                    tasks_clone.send(Task::SendErr(addr_from, err.clone()))
                        .unwrap_or_else(|e| error!("sending SendErr({addr_from}, {err:?}) to tasks failed, error {e}"))
                    }
            }).unwrap_or_else(|e| {
                let err = cli_ser::ServerErr::Sending(format!("{}, because the thread pool execution failed, error {e:?}", err_msg(addr_to)));
                tasks.send(Task::SendErr(addr_from, err.clone()))
                    .unwrap_or_else(|e| error!("sending SendErr({addr_from}, {err:?}) to tasks failed, error {e}"))
            });
        }
    }
    Ok(())
}

type Job = Box<dyn FnOnce() + Send + 'static>;

/// Structure for executing jobs in parallel on fixed number of threads.
///
/// Inspired by: <https://doc.rust-lang.org/book/ch20-02-multithreaded.html>
///
/// Warning: The thread pool can not recover from a poisoned mutex on the job channel.
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

    /// TODO: Can not use anyhow, because Box<dyn FnOnce() + Send + 'static>
    /// can not be shared between threads.
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
pub fn init_logging_stdout_and_file() -> anyhow::Result<WorkerGuard> {
    let term_layer = tracing_subscriber::fmt::layer().with_filter(LevelFilter::INFO);

    let file = fs::File::create(format!(
        "{}.log",
        Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
    ))
    .with_context(|| "Log file creation should be possible, please check your permissions.")?;
    let (non_blocking, guard) = tracing_appender::non_blocking(file);
    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(non_blocking)
        .with_filter(LevelFilter::TRACE);

    tracing_subscriber::registry()
        .with(term_layer)
        .with(file_layer)
        .init(); // sets itself as global default subscriber
    Ok(guard)
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
