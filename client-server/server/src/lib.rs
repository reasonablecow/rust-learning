//! # Server Executable
//!
//! Listens at a specified address and broadcasts every received message to all other connected clients.
//! See:
//! ```sh
//! cargo run -- --help
//! ```
//!
//! TODO: Test client disconnection.
use std::{net::SocketAddr, sync::Arc};

use anyhow::Context;
use dashmap::DashMap;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{
        tcp::{OwnedReadHalf, OwnedWriteHalf},
        TcpListener,
    },
    sync::mpsc::{self, Receiver, Sender},
};
use tracing::{error, info};

use crate::Task::*;
use cli_ser::{write_bytes, Message, ServerErr};

pub const HOST_DEFAULT: [u8; 4] = [127, 0, 0, 1];
pub const PORT_DEFAULT: u16 = 11111;

pub fn address_default() -> SocketAddr {
    SocketAddr::from((HOST_DEFAULT, PORT_DEFAULT))
}

/// Tasks to be initially queued at the server and addressed later.
#[derive(Debug)]
enum Task {
    Broadcast(SocketAddr, Message),
    SendErr(SocketAddr, Message),
    CloseStream(SocketAddr),
}

struct MsgWrap {
    msg: Message,
    addr_from: Option<SocketAddr>,
}

/// Channels to tasks which writes to specified Address over TCP.
type Senders = DashMap<SocketAddr, Sender<MsgWrap>>;

/// Asynchronously listen for incoming clients, reads their messages and broadcasts them.
///
/// The server is bound to a specified address.
/// In the main loop, the server processes tasks one at a time from its queue.
/// The server is written as if it should run forever.
pub async fn run(address: SocketAddr) -> anyhow::Result<()> {
    let (task_producer, mut task_consumer) = mpsc::channel(1024);
    let clients: Arc<Senders> = Arc::new(DashMap::new());
    let _listener = tokio::spawn(client_listener(address, task_producer, clients.clone()));
    while let Some(task) = task_consumer.recv().await {
        match task {
            Broadcast(addr_from, msg) => {
                info!("broadcasting message from {addr_from:?}");
                for client in clients.iter() {
                    let (addr_to, msg_channel) = (client.key(), client.value());
                    if addr_from != *addr_to {
                        _ = msg_channel
                            .send(MsgWrap {
                                msg: msg.clone(),
                                addr_from: Some(addr_from),
                            })
                            .await;
                    }
                }
            }
            SendErr(addr, err) => {
                if let Some(channel) = clients.get(&addr) {
                    _ = channel
                        .send(MsgWrap {
                            msg: err,
                            addr_from: None,
                        })
                        .await;
                }
            }
            CloseStream(addr) => {
                _ = clients.remove(&addr);
            }
        }
    }
    Ok(())
}

async fn client_listener(
    address: SocketAddr,
    tasks: Sender<Task>,
    clients: Arc<Senders>,
) -> anyhow::Result<()> {
    let listener = TcpListener::bind(address)
        .await
        .with_context(|| format!("Listening at {address:?} failed."))?;
    info!("Server is listening at {address:?}");
    loop {
        match listener.accept().await {
            Ok((socket, addr)) => {
                info!("incoming {addr:?}");
                let (reader, writer) = socket.into_split();
                let (msg_producer, msg_consumer) = mpsc::channel(128);
                clients.insert(addr, msg_producer);
                tokio::spawn(read_in_loop(addr, tasks.clone(), reader));
                tokio::spawn(write_each_msg(addr, tasks.clone(), writer, msg_consumer));
            }
            Err(e) => error!("incoming stream error: {e:?}"),
        }
    }
}

/// Reads messages from `addr` in a loop, sends them to `tasks` queue.
///
/// Panics when sending a task to `tasks` fails.
async fn read_in_loop(addr: SocketAddr, tasks: Sender<Task>, mut reader: OwnedReadHalf) {
    let tasks_broken_panic = "Emergency! Task queue stopped working!";
    let e = loop {
        let mut buf = match reader.read_u32().await {
            Ok(len) => vec![0u8; len as usize],
            Err(e) => break e,
        };
        if let Err(e) = reader.read_exact(&mut buf).await {
            break e;
        };
        let task = match Message::deserialize(&buf) {
            Ok(msg) => Broadcast(addr, msg),
            Err(_) => SendErr(
                addr,
                Message::from(ServerErr::Receiving("Message decoding failed!".to_string())),
            ),
        };
        tasks.send(task).await.expect(tasks_broken_panic);
    };
    error!("Reading messages of {addr} failed! Error: {e:?}");
    tasks
        .send(CloseStream(addr))
        .await
        .expect(tasks_broken_panic);
}

/// Writes every coming message from `messages` into `writer`.
///
/// Panics when sending a task to `tasks` fails.
async fn write_each_msg(
    addr: SocketAddr,
    tasks: Sender<Task>,
    mut writer: OwnedWriteHalf,
    mut messages: Receiver<MsgWrap>,
) {
    while let Some(MsgWrap { msg, addr_from }) = messages.recv().await {
        let bytes = match msg.serialize() {
            Ok(bytes) => bytes,
            Err(e) => {
                error!("Serialization of message from {addr_from:?} failed! Error {e}");
                continue;
            }
        };
        if let Err(_e) = write_bytes(&mut writer, &bytes).await {
            if let Some(addr_for_err) = addr_from {
                let msg = Message::from(ServerErr::Sending(format!(
                    "Sending a message to {addr} failed!"
                )));
                let task = SendErr(addr_for_err, msg);
                tasks.send(task).await.expect("Task queue stopped working!");
            };
            tasks
                .send(CloseStream(addr))
                .await
                .expect("Task queued stopped working!");
            break;
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn test_run_1_sec() {
        let server_thread = tokio::spawn(run(SocketAddr::from((HOST_DEFAULT, PORT_DEFAULT))));
        std::thread::sleep(Duration::from_secs(1));
        assert!(!server_thread.is_finished());
    }
}
