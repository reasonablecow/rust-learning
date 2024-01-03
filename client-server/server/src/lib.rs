//! # Server Executable
//!
//! Listens at a specified address, reads from and writes to connected clients.
//!
//! You need to set up a database first, see [db].
//!
//! For options run:
//! ```sh
//! cargo run -- --help
//! ```
//!
//! TODO: Test client disconnection.
use std::{net::SocketAddr, sync::Arc};

use anyhow::Context;
use chrono::{offset::Utc, SecondsFormat};
use dashmap::DashMap;
use tokio::{
    net::{
        tcp::{OwnedReadHalf, OwnedWriteHalf},
        TcpListener, TcpStream,
    },
    sync::mpsc::{self, Receiver, Sender},
};
use tracing::{debug, error, info, warn};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{
    filter::LevelFilter, prelude::__tracing_subscriber_SubscriberExt, util::SubscriberInitExt,
    Layer,
};

mod db;

use crate::Task::*;
use cli_ser::{cli, ser, Error::DisconnectedStream, Messageable, User};

pub const HOST_DEFAULT: [u8; 4] = [127, 0, 0, 1];
pub const PORT_DEFAULT: u16 = 11111;

pub fn address_default() -> SocketAddr {
    SocketAddr::from((HOST_DEFAULT, PORT_DEFAULT))
}

/// Tasks to be initially queued at the server and addressed later.
#[derive(Debug, Clone)]
enum Task {
    Broadcast(SocketAddr, ser::Msg),
    SendErr(SocketAddr, ser::Msg),
}

/// Channels to tasks which writes to specified Address over TCP.
type Senders = DashMap<SocketAddr, Sender<ser::Msg>>;

pub struct Server {
    address: SocketAddr,
    db: Arc<db::Database>,
}
impl Server {
    /// Builds the server, especially database initialization, takes time."
    pub async fn build(address: impl Into<SocketAddr>) -> anyhow::Result<Self> {
        let address = address.into();
        let db = Arc::new(db::Database::try_new().await?);
        Ok(Server { address, db })
    }

    /// Runs the server, connections should be accepted immediately.
    pub async fn run(self) -> anyhow::Result<()> {
        run(self).await
    }
}

/// Asynchronously listen for clients, reads their messages and acts accordingly.
///
/// The server is bound to a specified address.
/// In the main loop, the server processes tasks one at a time from its queue.
/// The server is written as if it should run forever.
async fn run(server: Server) -> anyhow::Result<()> {
    let Server { address, db } = server;
    let (task_producer, mut task_consumer) = mpsc::channel(1024);
    let clients: Arc<Senders> = Arc::new(DashMap::new());
    let listener = tokio::spawn(client_listener(address, task_producer, clients.clone(), db));
    while let Some(task) = task_consumer.recv().await {
        match task {
            Broadcast(addr_from, msg) => {
                info!("broadcasting \"{msg}\" from {addr_from:?}");
                for client in clients.iter() {
                    let (addr_to, msg_channel) = (client.key(), client.value());
                    if addr_from != *addr_to {
                        match msg_channel.send(msg.clone()).await {
                            Ok(_) => debug!("broadcasting to {addr_to:?}"),
                            Err(e) => warn!("broadcasting to {addr_to:?} failed, error {e}"),
                        }
                    }
                }
            }
            SendErr(addr, msg) => {
                if let Some(channel) = clients.get(&addr) {
                    if let Err(e) = channel.send(msg.clone()).await {
                        warn!("Sending error msg {msg:?} to {addr} failed! Error: {e:?}");
                    }
                }
            }
        }
    }
    listener.await?
}

/// Listens for connections, spawns task to handle each client.
async fn client_listener(
    address: SocketAddr,
    tasks: Sender<Task>,
    clients: Arc<Senders>,
    db: Arc<db::Database>,
) -> anyhow::Result<()> {
    let listener = TcpListener::bind(address)
        .await
        .with_context(|| format!("Listening at {address:?} failed."))?;
    info!("Server is listening at {address:?}");
    loop {
        match listener.accept().await {
            Ok((mut socket, addr)) => {
                info!("incoming {addr:?}");
                {
                    let (tasks, clients, db) = (tasks.clone(), clients.clone(), db.clone());
                    tokio::spawn(async move {
                        match authenticate(&mut socket, db).await {
                            Ok(user) => {
                                if let Err(e) =
                                    manage_client(addr, user, socket, clients, tasks).await
                                {
                                    error!("Managing client at {addr} failed! Error {e:#}");
                                }
                            }
                            Err(e) => {
                                error!("Authenticating the client at {addr} failed! Error {e:#}")
                            }
                        }
                    });
                }
            }
            Err(e) => error!("incoming stream error: {e:?}"),
        }
    }
}

/// Adds the client to `clients`, reads from and writes to it, then removes it from `clients`.
async fn manage_client(
    addr: SocketAddr,
    user: User,
    socket: TcpStream,
    clients: Arc<Senders>,
    tasks: Sender<Task>,
) -> anyhow::Result<()> {
    let (reader, writer) = socket.into_split();

    let (msg_producer, msg_consumer) = mpsc::channel(128);
    let writer_task = tokio::spawn(write_each_msg(msg_consumer, writer));

    clients.insert(addr, msg_producer);
    let reader_res = read_in_loop(addr, user, reader, tasks.clone()).await;
    clients
        .remove(&addr)
        .with_context(|| "Removing disconnected client \"{addr}\" from clients failed!")?;

    reader_res.with_context(|| "Reading messages at {addr} failed!")?;
    writer_task
        .await
        .context("Writer task should never panic, contact the implementer!")?;
    Ok(())
}

async fn authenticate(socket: &mut TcpStream, db: Arc<db::Database>) -> anyhow::Result<User> {
    let user = loop {
        let err = match cli::Msg::receive(socket).await? {
            cli::Msg::Auth(cli::Auth::LogIn(creds)) => match db.log_in(creds.clone()).await {
                Ok(()) => break creds.user,
                Err(db::Error::UserDoesNotExist(_)) => ser::Error::WrongUser,
                Err(db::Error::WrongPassword(_)) => ser::Error::WrongPassword,
                Err(e) => return Err(e.into()),
            },
            cli::Msg::Auth(cli::Auth::SignUp(creds)) => match db.sign_up(creds.clone()).await {
                Ok(()) => break creds.user,
                Err(db::Error::UsernameTaken(_)) => ser::Error::UsernameTaken,
                Err(e) => return Err(e.into()),
            },
            m => ser::Error::NotAuthenticated(m),
        };
        ser::Msg::Error(err).send(socket).await?;
    };
    ser::Msg::Authenticated
        .send(socket)
        .await
        .with_context(|| "Sending authentication confirmation failed!")?;
    Ok(user)
}

/// Receives messages from `reader` until disconnection, sends tasks to the `tasks` queue.
async fn read_in_loop(
    addr: SocketAddr,
    user: User,
    mut reader: OwnedReadHalf,
    tasks: Sender<Task>,
) -> anyhow::Result<()> {
    loop {
        let task = match cli::Msg::receive(&mut reader).await {
            Ok(cli::Msg::ToAll(data)) => Broadcast(
                addr,
                ser::Msg::DataFrom {
                    data,
                    from: user.clone(),
                },
            ),
            Ok(cli::Msg::Auth { .. }) => {
                SendErr(addr, ser::Msg::Error(ser::Error::AlreadyAuthenticated))
            }
            Err(DisconnectedStream(_)) => break Ok(()),
            Err(e) => SendErr(addr, ser::Msg::Error(ser::Error::ReceiveMsg(e.to_string()))),
        };
        tasks
            .send(task)
            .await
            .with_context(|| "Emergency! Task queue stopped working!")?;
    }
}

/// Writes every received message from `messages` into `writer`.
async fn write_each_msg(mut messages: Receiver<ser::Msg>, mut writer: OwnedWriteHalf) {
    while let Some(msg) = messages.recv().await {
        if let Err(e) = msg.send(&mut writer).await {
            error!("Writing the message {msg} to {writer:?} failed! Error {e}")
        }
    }
}

/// Subscribes to tracing (and logging), outputs to stdout and a log file.
///
/// Returns WorkerGuard which must be kept for the intended time of log capturing.
pub fn init_logging_stdout_and_file() -> anyhow::Result<WorkerGuard> {
    let term_layer = tracing_subscriber::fmt::layer().with_filter(LevelFilter::INFO);

    let file = std::fs::File::create(format!(
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

    #[tokio::test]
    async fn test_run_1_sec() {
        let server = Server::build((HOST_DEFAULT, PORT_DEFAULT)).await.unwrap();
        let server_thread = tokio::spawn(server.run());
        std::thread::sleep(Duration::from_secs(1));
        assert!(!server_thread.is_finished());
    }
}
