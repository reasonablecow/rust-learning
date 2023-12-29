//! # Server Executable
//!
//! Listens at a specified address and broadcasts every received message to all other connected clients.
//!
//! In order to set up the postgres database you can use:
//! ```sh
//! sudo docker run -p 5432:5432 --name pg -e POSTGRES_PASSWORD=pp -d postgres
//! ```
//!
//! See:
//! ```sh
//! cargo run -- --help
//! ```
//!
//! TODO: Move database to separate file
//! TODO: During an authentication, there can not be pause between
//! select from users and insert into users,
//! otherwise two users with the same name can be created at the same time.
//! TODO: std mutex is preferred over tokio mutex, but pool.query needs .await
//! "The primary use case for the async mutex is to provide shared mutable access to IO resources such as a database connection."
//! -- <https://docs.rs/tokio/latest/tokio/sync/struct.Mutex.html>
//! possiblility to refactor the database with actor model.
//! TODO: Test client disconnection.
//! TODO: Handle listener task.
use std::{net::SocketAddr, sync::Arc};

use anyhow::Context;
use chrono::{offset::Utc, SecondsFormat};
use dashmap::DashMap;
use sqlx::postgres::{PgPool, PgPoolOptions};
use tokio::{
    join,
    net::{
        tcp::{OwnedReadHalf, OwnedWriteHalf},
        TcpListener, TcpStream,
    },
    sync::{
        mpsc::{self, Receiver, Sender},
        Mutex,
    },
};
use tracing::{debug, error, info, warn};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{
    filter::LevelFilter, prelude::__tracing_subscriber_SubscriberExt, util::SubscriberInitExt,
    Layer,
};

use crate::Task::*;
use cli_ser::{cli, ser, Data, Error::DisconnectedStream, Messageable, ServerErr};

pub const HOST_DEFAULT: [u8; 4] = [127, 0, 0, 1];
pub const PORT_DEFAULT: u16 = 11111;
const CONN_STR: &str = "postgres://postgres:pp@localhost:5432/postgres";

pub fn address_default() -> SocketAddr {
    SocketAddr::from((HOST_DEFAULT, PORT_DEFAULT))
}

/// Tasks to be initially queued at the server and addressed later.
#[derive(Debug, Clone)]
enum Task {
    Broadcast(SocketAddr, Data),
    SendErr(SocketAddr, ServerErr),
    CloseStream(SocketAddr),
}

enum DataOrErr {
    Data { from: SocketAddr, data: Data },
    Err(ServerErr),
}

/// Channels to tasks which writes to specified Address over TCP.
type Senders = DashMap<SocketAddr, Sender<DataOrErr>>;

#[derive(Debug, sqlx::FromRow)]
struct User {
    username: String,
    password: String,
}

const CREATE_USERS: &str = r#"
CREATE TABLE IF NOT EXISTS "users" (
  "id" bigserial PRIMARY KEY,
  "username" text NOT NULL,
  "password" text NOT NULL
);
"#;

async fn init_database() -> anyhow::Result<PgPool> {
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(CONN_STR)
        .await?;
    sqlx::query(CREATE_USERS).execute(&pool).await?;
    Ok(pool)
}

pub struct Server {
    address: SocketAddr,
    pool: Arc<Mutex<PgPool>>,
}
impl Server {
    /// Builds the server, especially database initialization, takes time."
    pub async fn build(address: impl Into<SocketAddr>) -> anyhow::Result<Self> {
        let address = address.into();
        let pool = Arc::new(Mutex::new(init_database().await?));
        Ok(Server { address, pool })
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
    let (task_producer, mut task_consumer) = mpsc::channel(1024);
    let clients: Arc<Senders> = Arc::new(DashMap::new());
    let _listener = tokio::spawn(client_listener(server, task_producer, clients.clone()));
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
    Ok(())
}

async fn client_listener(
    server: Server,
    tasks: Sender<Task>,
    clients: Arc<Senders>,
) -> anyhow::Result<()> {
    let (address, pool) = (server.address, server.pool);
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
                    pool.clone(),
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
    pool: Arc<Mutex<PgPool>>,
) -> anyhow::Result<()> {
    let (mut reader, mut writer) = socket.into_split();
    if let Err(e) = authenticate(&mut reader, &mut writer, pool).await {
        error!("{e}");
        return Err(e);
    }
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
    pool: Arc<Mutex<PgPool>>,
) -> anyhow::Result<()> {
    ser::Msg::Info("Please log in with existing user or create a new one.".to_string())
        .send(writer)
        .await?;
    loop {
        match cli::Msg::receive(reader).await? {
            cli::Msg::Auth { username, password } => {
                let pool = pool.lock().await;
                match sqlx::query_as::<_, User>("SELECT * FROM users WHERE username = $1")
                    .bind(username.clone())
                    .fetch_optional(&*pool)
                    .await
                    .with_context(|| "Querying the database for authentication failed.")?
                {
                    Some(user) => {
                        // TODO - https://docs.rs/argon2/latest/argon2/
                        if password == user.password {
                            ser::Msg::Info(format!(
                                "Hi {}, logging in was successful!",
                                user.username
                            ))
                            .send(writer)
                            .await?;
                            break Ok(());
                        }
                        ser::Msg::Info(format!(
                            "Incorrect password for user \"{}\", try again!",
                            user.username
                        ))
                        .send(writer)
                        .await?;
                    }
                    None => {
                        sqlx::query("INSERT INTO users (username, password) VALUES ($1, $2);")
                            .bind(username.clone())
                            .bind(password.clone()) // TODO
                            .execute(&*pool)
                            .await?;
                        ser::Msg::Info(format!("Welcome {username}, your account was created!"))
                            .send(writer)
                            .await?;
                        break Ok(());
                    }
                }
            }
            _ => {
                ser::Msg::Info("You need to log in before sending messages, try again.".to_string())
                    .send(writer)
                    .await?
            }
        }
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
            Err(e) => SendErr(addr, ServerErr::Receiving(format!("{e:?}"))),
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
                            ServerErr::Sending(format!("Sending a message to {addr} failed!"));
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
