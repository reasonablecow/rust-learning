use std::net::{IpAddr, SocketAddr};

use anyhow::Context;
use chrono::{offset::Utc, SecondsFormat};
use clap::Parser;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{
    filter::LevelFilter, prelude::__tracing_subscriber_SubscriberExt, util::SubscriberInitExt,
    Layer,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let address = Args::parse_to_address()?;
    let _log_file_guard = init_logging_stdout_and_file()?;
    server::run(address).await
}

/// Server executable, listens at specified address and broadcasts messages to all connected clients.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// Server host
    #[arg(long, default_value_t = format!("{}", IpAddr::from(server::HOST_DEFAULT)))]
    host: String,

    /// Server port
    #[arg(short, long, default_value_t = server::PORT_DEFAULT)]
    port: u16,
}
impl Args {
    pub fn parse_to_address() -> anyhow::Result<SocketAddr> {
        let args = Self::parse();
        let ip: IpAddr = args.host.parse()?;
        Ok(SocketAddr::from((ip, args.port)))
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
