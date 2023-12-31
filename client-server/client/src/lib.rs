//! # Client Executable
//!
//! See:
//! ```sh
//! cargo run -- --help
//! ```
use std::{net::SocketAddr, path::PathBuf};

use anyhow::{anyhow, Context};
use regex::Regex;
use tokio::{
    io::{self, AsyncReadExt},
    net::TcpStream,
    select,
    sync::{mpsc, oneshot},
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
    /// Sends messages (read form the terminal) to the server, and prints received ones.
    ///
    /// Spawns tcp receiver task and stdin reader thread.
    /// Sends messages received from the reader thread channel to the server.
    pub async fn run(self) -> anyhow::Result<()> {
        let socket = TcpStream::connect(self.addr).await.with_context(|| {
            "Connection to the server failed, please make sure the server is running."
        })?;

        let (reader, mut writer) = io::split(socket);

        // Channel to indicate to stop waiting for messages.
        let (send_quit, recv_quit) = oneshot::channel();

        // Reads messages from the server in the background.
        let receiver = tokio::spawn(self.clone().receive_in_loop(reader, recv_quit));

        // Parses and executes commands given by the user.
        let (msg_producer, mut msg_consumer) = mpsc::channel(128);
        let stdin_reader =
            std::thread::spawn(move || Self::parse_commands_from_stdin(msg_producer, send_quit));
        while let Some(msg) = msg_consumer.recv().await {
            msg.send(&mut writer)
                .await
                .with_context(|| "sending your message to the server failed")?;
        }

        // Returns error when stdin_reader failed (couldn't send quit to the receiver).
        // .join()'s Err variant does not implement Error trait -> .expect.
        stdin_reader
            .join()
            .expect("stdin command parser should never panic")
            .with_context(|| "Command reader failed.")?;

        // Wait for the receiver to finish its job.
        receiver
            .await?
            .with_context(|| "Receiver went through an unrecoverable error")?;

        Ok(())
    }

    /// Receives and processes messages from the server until quit message comes.
    async fn receive_in_loop<S>(
        self,
        mut socket: S,
        mut quit: oneshot::Receiver<()>,
    ) -> anyhow::Result<()>
    where
        S: AsyncReadExt + std::marker::Unpin + std::marker::Send,
    {
        println!("Please .login with user and password or .signup to create a new one.");
        loop {
            select!(
                msg = ser::Msg::receive(&mut socket) => {
                    self.process_msg(msg.with_context(|| "reading a message from server failed")?).await;
                }
                _ = &mut quit => break Ok(()),
            )
        }
    }

    /// Processes given message, either prints, or writes to a file.
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
            ser::Msg::Authenticated => println!("Welcome!"),
            ser::Msg::Error(ser::Error::WrongPassword) => {
                eprintln!("Given password is not correct")
            }
            ser::Msg::Error(ser::Error::WrongUser) => {
                eprintln!("The user does not exist, you can create it with a .signup")
            }
            ser::Msg::Error(ser::Error::UsernameTaken) => {
                eprintln!("Unfortunately this username is already taken, choose another one.")
            }
            ser::Msg::Error(err) => eprintln!("Error: {err:?}"),
        };
    }

    /// Reads commands from standard input and sends messages or a quit signal over the appropriate channel.
    ///
    /// The [tokio documentation](https://docs.rs/tokio_wasi/latest/tokio/io/fn.stdin.html)
    /// says you should spawn a separate thread.
    ///
    /// I had a problem to see anywhere implemented that way, the
    /// [Google Comprehensive Rust - Chat Application](https://google.github.io/comprehensive-rust/exercises/concurrency/chat-app.html)
    /// uses BufReader, however it requires at least one "enter" stroke for tests.
    ///
    /// ["For technical reasons, stdin is implemented by using an ordinary blocking read on a separate thread, and it is impossible to cancel that read. This can make shutdown of the runtime hang until the user presses enter."](https://docs.rs/tokio/latest/tokio/io/struct.Stdin.html)
    fn parse_commands_from_stdin(
        messages: mpsc::Sender<cli::Msg>,
        quit: oneshot::Sender<()>,
    ) -> anyhow::Result<()> {
        for line in std::io::stdin().lines() {
            match Command::try_from(&*line?) {
                Ok(Command::Quit) => {
                    quit.send(()).map_err(|_| {
                        anyhow!("Sending a quit signal to the message receiver failed")
                    })?;
                    break;
                }
                Ok(Command::Msg(msg)) => messages
                    .blocking_send(msg)
                    .with_context(|| "Sending a parsed message failed")?,
                Err(e) => eprintln!("Couldn't create your message (error: {e:?})"),
            };
        }
        Ok(())
    }
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
        let reg_log_in =
            Regex::new(r"^\s*\.login\s+(?<name>\S+.*)\s+(?<pswd>\S+.*)\s*$").expect(err_msg);
        let reg_sign_up =
            Regex::new(r"^\s*\.signup\s+(?<name>\S+.*)\s+(?<pswd>\S+.*)\s*$").expect(err_msg);

        let cmd = if reg_quit.is_match(value) {
            Command::Quit
        } else if let Some((_, [file])) = reg_file.captures(value).map(|caps| caps.extract()) {
            Command::Msg(cli::Msg::file_from_path(file)?)
        } else if let Some((_, [image])) = reg_image.captures(value).map(|caps| caps.extract()) {
            Command::Msg(cli::Msg::img_from_path(image)?)
        } else if let Some((_, [name, pswd])) =
            reg_log_in.captures(value).map(|caps| caps.extract())
        {
            Command::Msg(cli::Msg::Auth(cli::Auth::LogIn(cli::User {
                username: name.to_string(),
                password: pswd.to_string(),
            })))
        } else if let Some((_, [name, pswd])) =
            reg_sign_up.captures(value).map(|caps| caps.extract())
        {
            Command::Msg(cli::Msg::Auth(cli::Auth::SignUp(cli::User {
                username: name.to_string(),
                password: pswd.to_string(),
            })))
        } else {
            Command::Msg(cli::Msg::Data(Data::Text(value.to_string())))
        };
        Ok(cmd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::time::Duration;

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
