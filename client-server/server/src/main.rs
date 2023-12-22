use std::net::{IpAddr, SocketAddr};

use clap::Parser;

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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let address = Args::parse_to_address()?;
    let _log_file_guard = server::init_logging_stdout_and_file()?;
    server::run(address).await
}
