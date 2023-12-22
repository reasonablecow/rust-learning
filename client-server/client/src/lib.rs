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

use cli_ser::{cli, ser, Data, Messageable};

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

    async fn receive_in_loop<S>(
        self,
        mut socket: S,
        mut recv_quit: Receiver<()>,
    ) -> anyhow::Result<()>
    where
        S: AsyncReadExt + std::marker::Unpin + std::marker::Send,
    {
        loop {
            select!(
                msg = ser::Msg::receive(&mut socket) => {
                    self.process_msg(msg.with_context(|| "reading a message from server failed")?).await;
                }
                _ = &mut recv_quit => break Ok(()),
            )
        }
    }

    async fn process_msg(&self, msg: ser::Msg) {
        match msg {
            ser::Msg::DataFrom {
                data: Data::Text(text),
                from,
            } => println!("{from}: {text}"),
            ser::Msg::DataFrom {
                data: Data::File(f),
                from,
            } => {
                println!("Received {:?} from {from}", f.name());
                f.save(&self.file_dir).unwrap_or_else(|e| {
                    eprintln!("...saving the file \"{:?}\" failed! Err: {:?}", f.name(), e)
                });
            }
            ser::Msg::DataFrom {
                data: Data::Image(image),
                from,
            } => {
                println!("Received image from {from}...");
                match if self.save_png {
                    image.save_as_png(&self.img_dir)
                } else {
                    image.save(&self.img_dir)
                } {
                    Ok(path) => println!("...image was saved to {:?}", path),
                    Err(e) => eprintln!("...saving the image failed! Err: {:?}", e),
                }
            }
            ser::Msg::Info(text) => println!("{text}"),
            ser::Msg::Error(err) => eprintln!("received error: \"{err:?}\""),
        };
    }

    ///
    ///
    /// The [tokio documentation](https://docs.rs/tokio_wasi/latest/tokio/io/fn.stdin.html)
    /// says you should spawn a separate thread.
    /// I had a problem to see it anywhere used like it, the
    /// <https://google.github.io/comprehensive-rust/exercises/concurrency/chat-app.html>
    /// uses BufReader.
    ///
    /// TODO <https://docs.rs/tokio/latest/tokio/io/struct.Stdin.html>
    /// For technical reasons, stdin is implemented by using an ordinary blocking read on a separate thread, and it is impossible to cancel that read. This can make shutdown of the runtime hang until the user presses enter.
    async fn send_from_stdin<S>(mut socket: S, send_quit: Sender<()>) -> anyhow::Result<()>
    where
        S: AsyncWriteExt + std::marker::Unpin + std::marker::Send,
    {
        let mut lines = BufReader::new(tokio::io::stdin()).lines(); // TODO

        while let Some(line) = lines
            .next_line()
            .await
            .with_context(|| "reading your command failed")?
        {
            match Command::try_from(&*line) {
                Ok(Command::Quit) => {
                    println!("Logging out...");
                    // Time for messages which were sent but were not receivable yet.
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    send_quit.send(()).map_err(|_| {
                        anyhow!("Sending a quit signal to the message receiver failed")
                    })?;
                    break;
                }
                Ok(Command::Msg(msg)) => msg
                    .send(&mut socket)
                    .await
                    .with_context(|| "sending your message to the server failed")?,
                Err(e) => eprintln!("Couldn't create your message (error: {e:?})"),
            };
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
    Msg(cli::Msg),
}
/// Converts the first line of the borrowed str to Command, rest is ignored.
impl TryFrom<&str> for Command {
    type Error = anyhow::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let err_msg = "unexpected regex error, contact the crate implementer";
        let reg_quit = Regex::new(r"^\s*\.quit\s*$").expect(err_msg);
        let reg_file = Regex::new(r"^\s*\.file\s+(?<file>\S+.*)\s*$").expect(err_msg);
        let reg_image = Regex::new(r"^\s*\.image\s+(?<image>\S+.*)\s*$").expect(err_msg);
        let reg_auth =
            Regex::new(r"^\s*\.login\s+(?<name>\S+.*)\s+(?<pswd>\S+.*)\s*$").expect(err_msg);

        let cmd = if reg_quit.is_match(value) {
            Command::Quit
        } else if let Some((_, [file])) = reg_file.captures(value).map(|caps| caps.extract()) {
            Command::Msg(cli::Msg::file_from_path(file)?)
        } else if let Some((_, [image])) = reg_image.captures(value).map(|caps| caps.extract()) {
            Command::Msg(cli::Msg::img_from_path(image)?)
        } else if let Some((_, [name, pswd])) = reg_auth.captures(value).map(|caps| caps.extract())
        {
            Command::Msg(cli::Msg::Auth {
                username: name.to_string(),
                password: pswd.to_string(),
            })
        } else {
            Command::Msg(cli::Msg::Data(Data::Text(value.to_string())))
        };
        Ok(cmd)
    }
}

// TODO
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_from_str_quit() {
        assert_eq!(Command::Quit, Command::try_from(".quit").unwrap());
        assert_eq!(
            Command::Quit,
            Command::try_from("      .quit      ").unwrap()
        );
    }

    #[test]
    fn command_from_str_file() {
        let path = "Cargo.toml";
        assert_eq!(
            Command::Msg(cli::Msg::file_from_path(path).unwrap()),
            Command::try_from(&*format!(".file {path}")).unwrap()
        );
    }

    #[test]
    fn command_from_str_image() {
        let path = "../example-images/rustacean-orig-noshadow.png";
        assert_eq!(
            Command::Msg(cli::Msg::img_from_path(path).unwrap()),
            Command::try_from(&*format!(".image {path}")).unwrap()
        );
    }

    #[test]
    fn command_from_str_other() {
        for s in [". quit", ".quit      s", "a   .quit "] {
            assert_eq!(
                Command::Msg(cli::Msg::Data(Data::Text(s.to_string()))),
                Command::try_from(s).unwrap()
            );
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
