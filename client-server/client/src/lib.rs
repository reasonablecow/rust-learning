//! # Client Executable
//!
//! See:
//! ```sh
//! cargo run -- --help
//! ```
//!
//! TODO: buffered
use std::{net::SocketAddr, path::PathBuf, time::Duration};

use anyhow::{anyhow, Context};
use regex::Regex;
use tokio::{
    io::{self, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
    select,
    sync::oneshot::{self, Receiver, Sender},
};

use cli_ser::Message;

pub const HOST_DEFAULT: [u8; 4] = [127, 0, 0, 1];
pub const PORT_DEFAULT: u16 = 11111;

// Idea: maybe implement std Default for this...
#[derive(Clone)]
pub struct Client {
    pub file_dir: PathBuf,
    pub img_dir: PathBuf,
    pub addr: SocketAddr,
    pub save_png: bool,
}
impl Client {
    /// Client's main function.
    ///
    /// The main thread in a loop executes commands given by the user.
    /// Another thread is spawned in the background to receive messages from the server.
    pub async fn run(self) -> anyhow::Result<()> {
        let socket = TcpStream::connect(self.addr).await.with_context(|| {
            "Connection to the server failed, please make sure the server is running."
        })?;

        let (reader, writer) = io::split(socket);

        // Channel to indicate to stop waiting for messages.
        let (send_quit, recv_quit) = oneshot::channel();

        // Reads messages from the server in the background.
        let receiver = tokio::spawn(self.clone().receive_in_loop(reader, recv_quit));

        // Parses and executes commands given by the user.
        Self::send_from_stdin(writer, send_quit).await?;

        // Wait for the receiver to finish its job.
        receiver
            .await
            // // thread::Error doesn't implement Sync -> can not be used with anyhow
            // .expect("Message receiver crashed even though it never should, contact the implementer!")
            .with_context(|| "Receiver went through an unrecoverable error")??;
        println!("Good bye!");
        Ok(())
    }

    async fn receive_in_loop(
        self,
        mut socket: (impl AsyncReadExt + std::marker::Unpin),
        mut recv_quit: Receiver<()>,
    ) -> anyhow::Result<()> {
        loop {
            select!(
                msg = Message::receive(&mut socket) => {
                    self.process_msg(msg.with_context(|| "reading a message from server failed")?).await;
                }
                _ = &mut recv_quit => break Ok(()),
            )
        }
    }

    async fn process_msg(&self, msg: Message) {
        match msg {
            Message::Text(text) => println!("{}", text),
            Message::File(f) => {
                println!("Received {:?}", f.name());
                f.save(&self.file_dir).unwrap_or_else(|e| {
                    eprintln!("...saving the file \"{:?}\" failed! Err: {:?}", f.name(), e)
                });
            }
            Message::Image(image) => {
                println!("Received image...");
                match if self.save_png {
                    image.save_as_png(&self.img_dir)
                } else {
                    image.save(&self.img_dir)
                } {
                    Ok(path) => println!("...image was saved to {:?}", path),
                    Err(e) => eprintln!("...saving the image failed! Err: {:?}", e),
                }
            }
            Message::ServerErr(err) => eprintln!("received error: \"{err:?}\""),
        };
    }

    ///
    ///
    /// The [tokio documentation](https://docs.rs/tokio_wasi/latest/tokio/io/fn.stdin.html)
    /// says you should spawn a separate thread.
    /// I had a problem to see it anywhere used like it, the
    /// <https://google.github.io/comprehensive-rust/exercises/concurrency/chat-app.html>
    /// uses BufReader.
    async fn send_from_stdin(
        mut socket: (impl AsyncWriteExt + std::marker::Unpin),
        send_quit: Sender<()>,
    ) -> anyhow::Result<()> {
        let mut lines = BufReader::new(tokio::io::stdin()).lines();
        println!("Please type the command:");

        while let Some(line) = lines
            .next_line()
            .await
            .with_context(|| "reading your command failed")?
        {
            let msg = match Command::from(&*line) {
                Command::Quit => {
                    println!("Logging out...");
                    // Time for messages which were sent but were not receivable yet.
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    send_quit.send(()).map_err(|_| {
                        anyhow!("Sending a quit signal to the message receiver failed")
                    })?;
                    break;
                }
                cmd => Message::try_from(cmd)
                    .with_context(|| "Bug in a client code, contact the implementer")?,
            };
            msg.send(&mut socket)
                .await
                .with_context(|| "sending your message to the server failed")?;
        }
        Ok(())
    }
}

/// Reads line from standard input, strips ending newline if present.
///
/// todo: rewrite to strip_newline
fn _strip_newline(line: String) -> String {
    line.strip_suffix('\n').map(str::to_string).unwrap_or(line)
}

/// Commands useful for the client user.
#[derive(Debug, PartialEq)]
pub enum Command {
    Quit,
    File(String),
    Image(String),
    Other(String),
}
/// Converts the first line of the borrowed str to Command, rest is ignored.
impl From<&str> for Command {
    fn from(value: &str) -> Self {
        let err_msg = "unexpected regex error, contact the crate implementer";
        let reg_quit = Regex::new(r"^\s*\.quit\s*$").expect(err_msg);
        let reg_file = Regex::new(r"^\s*\.file\s+(?<file>\S+.*)\s*$").expect(err_msg);
        let reg_image = Regex::new(r"^\s*\.image\s+(?<image>\S+.*)\s*$").expect(err_msg);

        if reg_quit.is_match(value) {
            Command::Quit
        } else if let Some((_, [file])) = reg_file.captures(value).map(|caps| caps.extract()) {
            Command::File(file.to_string())
        } else if let Some((_, [image])) = reg_image.captures(value).map(|caps| caps.extract()) {
            Command::Image(image.to_string())
        } else {
            Command::Other(value.to_string())
        }
    }
}
impl TryFrom<Command> for Message {
    type Error = anyhow::Error;

    fn try_from(value: Command) -> Result<Self, <cli_ser::Message as TryFrom<Command>>::Error> {
        match value {
            Command::Quit => Err(anyhow!(
                "A Massage can not be constructed from a Quit command!"
            )),
            Command::Other(s) => Ok(Message::Text(s)),
            Command::File(path) => Ok(Message::file_from_path(path)?),
            Command::Image(path) => Ok(Message::img_from_path(path)?),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_from_str_quit() {
        assert_eq!(Command::Quit, Command::from(".quit"));
        assert_eq!(Command::Quit, Command::from("      .quit      "));
    }

    #[test]
    fn command_from_str_file() {
        let f = "a.txt";
        assert_eq!(
            Command::File(String::from(f)),
            Command::from(&*format!(".file {}", f))
        );
    }

    #[test]
    fn command_from_str_image() {
        let i = "i.png";
        assert_eq!(
            Command::Image(String::from(i)),
            Command::from(&*format!(".image {}", i))
        );
    }

    #[test]
    fn command_from_str_other() {
        for s in [". quit", ".quit      s", "a   .quit "] {
            assert_eq!(Command::Other(String::from(s)), Command::from(s));
        }
    }

    /// TODO: the test doesn't work without a single keyboard press
    #[tokio::test]
    async fn test_run() {
        let addr = SocketAddr::from((HOST_DEFAULT, PORT_DEFAULT));
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .expect("TCP listener creation should not fail.");
        let server_thread = tokio::spawn(async move {
            let mut v: Vec<TcpStream> = Vec::new();
            loop {
                let (socket, _) = listener.accept().await.expect("Accepting client failed!");
                v.push(socket);
            }
        });
        let client_thread = tokio::spawn(
            Client {
                img_dir: PathBuf::from("imgs"),
                file_dir: PathBuf::from("fls"),
                addr,
                save_png: true,
            }
            .run(),
        );
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert!(!server_thread.is_finished());
        assert!(!client_thread.is_finished());
    }
}
