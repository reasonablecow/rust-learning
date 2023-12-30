//! # Server Executable
//!
//! Listens at a specified address and broadcasts every received message to all other connected clients.
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
    join,
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
use cli_ser::{cli, ser, Data, Error::DisconnectedStream, Messageable};

pub const HOST_DEFAULT: [u8; 4] = [127, 0, 0, 1];
pub const PORT_DEFAULT: u16 = 11111;

pub fn address_default() -> SocketAddr {
    SocketAddr::from((HOST_DEFAULT, PORT_DEFAULT))
}

/// Tasks to be initially queued at the server and addressed later.
#[derive(Debug, Clone)]
enum Task {
    Broadcast(SocketAddr, Data),
    SendErr(SocketAddr, ser::Error),
    CloseStream(SocketAddr),
}

enum DataOrErr {
    Data { from: SocketAddr, data: Data },
    Err(ser::Error),
}

/// Channels to tasks which writes to specified Address over TCP.
type Senders = DashMap<SocketAddr, Sender<DataOrErr>>;

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

/// Asynchronously listen for incoming clients, reads their messages and broadcasts them.
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
                info!("broadcasting message from {addr_from:?}");
                for client in clients.iter() {
                    let (addr_to, msg_channel) = (client.key(), client.value());
                    if addr_from != *addr_to {
                        debug!("broadcasting to {addr_to:?}");
                        _ = msg_channel
                            .send(DataOrErr::Data {
                                data: msg.clone(),
                                from: addr_from,
                            })
                            .await;
                    }
                }
            }
            SendErr(addr, err) => {
                if let Some(channel) = clients.get(&addr) {
                    _ = channel.send(DataOrErr::Err(err)).await;
                }
            }
            CloseStream(addr) => {
                _ = clients.remove(&addr);
            }
        }
    }
    listener.await?
}

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
            Ok((socket, addr)) => {
                info!("incoming {addr:?}");
                tokio::spawn(process_socket(
                    addr,
                    tasks.clone(),
                    socket,
                    clients.clone(),
                    db.clone(),
                ));
            }
            Err(e) => error!("incoming stream error: {e:?}"),
        }
    }
}

async fn process_socket(
    addr: SocketAddr,
    tasks: Sender<Task>,
    socket: TcpStream,
    clients: Arc<Senders>,
    db: Arc<db::Database>,
) -> anyhow::Result<()> {
    let (mut reader, mut writer) = socket.into_split();
    if let Err(e) = authenticate(&mut reader, &mut writer, db)
        .await
        .with_context(|| format!("Authenticating the user at address {addr:?} failed"))
    {
        error!("{e:#}");
        return Err(e);
    }
    ser::Msg::Authenticated.send(&mut writer).await?;
    let (msg_producer, msg_consumer) = mpsc::channel(128);
    clients.insert(addr, msg_producer);
    join!(
        read_in_loop(addr, tasks.clone(), reader),
        write_each_msg(addr, tasks.clone(), writer, msg_consumer),
    );
    Ok(())
}

async fn authenticate(
    reader: &mut OwnedReadHalf,
    writer: &mut OwnedWriteHalf,
    db: Arc<db::Database>,
) -> anyhow::Result<()> {
    loop {
        let err = match cli::Msg::receive(reader).await? {
            cli::Msg::Auth(cli::Auth::LogIn(user)) => match db.log_in(user.clone()).await {
                Ok(()) => break Ok(()),
                Err(db::Error::UserDoesNotExist(_)) => ser::Error::WrongUser,
                Err(db::Error::WrongPassword(_)) => ser::Error::WrongPassword,
                Err(e) => return Err(e.into()),
            },
            cli::Msg::Auth(cli::Auth::SignUp(user)) => match db.sign_up(user.clone()).await {
                Ok(()) => break Ok(()),
                Err(db::Error::UsernameTaken(_)) => ser::Error::UsernameTaken,
                Err(e) => return Err(e.into()),
            },
            m => ser::Error::NotAuthenticated(m),
        };
        ser::Msg::Error(err).send(writer).await?;
    }
}

/// Reads messages from `addr` in a loop, sends them to `tasks` queue.
///
/// Panics when sending a task to `tasks` fails.
async fn read_in_loop(addr: SocketAddr, tasks: Sender<Task>, mut reader: OwnedReadHalf) {
    loop {
        let task = match cli::Msg::receive(&mut reader).await {
            Ok(cli::Msg::Data(data)) => Broadcast(addr, data),
            Ok(cli::Msg::Auth { .. }) => todo!(),
            Err(DisconnectedStream(_)) => CloseStream(addr),
            Err(e) => SendErr(addr, ser::Error::Receiving(format!("{e:?}"))),
        };
        tasks
            .send(task.clone())
            .await
            .expect("Emergency! Task queue stopped working!");
        if let CloseStream(_) = task {
            break;
        }
    }
}

/// Writes every coming message from `messages` into `writer`.
///
/// Panics when sending a task to `tasks` fails.
async fn write_each_msg(
    addr: SocketAddr,
    tasks: Sender<Task>,
    mut writer: OwnedWriteHalf,
    mut messages: Receiver<DataOrErr>,
) {
    while let Some(data_or_err) = messages.recv().await {
        match data_or_err {
            DataOrErr::Data { data, from } => {
                match (ser::Msg::DataFrom {
                    data,
                    from: from.to_string(),
                })
                .send(&mut writer)
                .await
                {
                    Err(DisconnectedStream(_)) => break,
                    Err(e) => {
                        let err =
                            ser::Error::Sending(format!("Sending a message to {addr} failed!"));
                        warn!("{err:?} because {e:?}");
                        let task = SendErr(from, err);
                        tasks.send(task).await.expect("Task queue stopped working!");
                    }
                    Ok(_) => {}
                }
            }
            DataOrErr::Err(e) => {
                let _ = ser::Msg::Error(e).send(&mut writer).await; // todo
            }
        };
    }
    tasks
        .send(CloseStream(addr))
        .await
        .expect("Task queued stopped working!");
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
