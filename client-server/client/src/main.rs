use std::{
    fs,
    net::{IpAddr, SocketAddr},
    path::PathBuf,
};

use anyhow::Context;
use clap::Parser;

use client::{Client, HOST_DEFAULT, PORT_DEFAULT};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let file_dir = PathBuf::from("files");
    let img_dir = PathBuf::from("images");
    fs::create_dir_all(&file_dir).with_context(|| "Directory for files couldn't be created")?;
    fs::create_dir_all(&img_dir).with_context(|| "Directory for images couldn't be created")?;

    let host: IpAddr = args.host.parse()?;
    let addr = SocketAddr::from((host, args.port));

    Client {
        file_dir,
        img_dir,
        addr,
        save_png: args.save_png,
    }
    .run()
    .await
}

/// Client executable, interactively sends messages to the specified server.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Server host
    #[arg(long, default_value_t = IpAddr::from(HOST_DEFAULT).to_string())]
    host: String,

    /// Server port
    #[arg(short, long, default_value_t = PORT_DEFAULT)]
    port: u16,

    /// Save all images as PNG.
    #[arg(short, long, default_value_t = false)]
    save_png: bool,
}
