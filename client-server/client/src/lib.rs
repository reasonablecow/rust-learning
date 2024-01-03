//! # Client Executable
//!
//! See:
//! ```sh
//! cargo run -- --help
//! ```
//!
//! TODO: Add ".help" or similar to see how to make messages right from the client.
use std::{net::SocketAddr, path::PathBuf, str::FromStr};

use anyhow::{anyhow, Context};
use regex::Regex;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    select,
    sync::{mpsc, oneshot},
};

use cli_ser::{cli, ser, Data, File, Image, Messageable};

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
    /// Spawns stdin reader thread, tcp sender and tcp receiver tasks.
    pub async fn run(self) -> anyhow::Result<()> {
        let (reader, writer) = TcpStream::connect(self.addr)
            .await
            .with_context(|| {
                "Connection to the server failed, please make sure the server is running."
            })?
            .into_split();
        // Channel to indicate to stop receiving for messages.
        let (quit_sender, quit_receiver) = oneshot::channel();
        // Channel to pass input read in blocking thread to the async handle task.
        let (input_producer, input_consumer) = mpsc::channel(128);

        let stdin_reader = std::thread::spawn(move || Self::read_stdin(input_producer));
        let msg_receiver = tokio::spawn(self.receive_in_loop(reader, quit_receiver));
        let msg_sender = tokio::spawn(Self::handle_input(input_consumer, writer, quit_sender));

        // Awaiting the msg_receiver first is important for crash to show up when it happens.
        msg_receiver
            .await?
            .with_context(|| "Receiver went through an unrecoverable error")?;
        msg_sender
            .await?
            .with_context(|| "Message sender crashed.")?;

        // thread .join()'s Err variant does not implement Error trait -> .expect.
        stdin_reader
            .join()
            .expect("stdin_reader thread should never panic")
            .with_context(|| "Reader of standard input crashed.")?;
        Ok(())
    }

    /// Reads lines from standard input and sends them over mpsc channel until a Quit is parsed.
    ///
    /// The [tokio documentation](https://docs.rs/tokio_wasi/latest/tokio/io/fn.stdin.html)
    /// says you should spawn a separate thread.
    ///
    /// I had a problem to see anywhere implemented that way, the
    /// [Google Comprehensive Rust - Chat Application](https://google.github.io/comprehensive-rust/exercises/concurrency/chat-app.html)
    /// uses BufReader, however it requires at least one "enter" stroke for tests.
    ///
    /// ["For technical reasons, stdin is implemented by using an ordinary blocking read on a separate thread,
    /// and it is impossible to cancel that read.
    /// This can make shutdown of the runtime hang until the user presses enter."](https://docs.rs/tokio/latest/tokio/io/struct.Stdin.html)
    fn read_stdin(sender: mpsc::Sender<String>) -> anyhow::Result<()> {
        for line in std::io::stdin().lines() {
            let line = line.with_context(|| "Reading a line from stdin failed.")?;
            if line.parse::<Quit>().is_ok() {
                break;
            }
            sender
                .blocking_send(line)
                .with_context(|| "Sending a line over the channel failed.")?;
        }
        Ok(())
    }

    /// Receives and processes messages from the server until quit message comes.
    async fn receive_in_loop<S>(
        self,
        mut reader: S,
        mut quit: oneshot::Receiver<()>,
    ) -> anyhow::Result<()>
    where
        S: AsyncReadExt + std::marker::Unpin + std::marker::Send,
    {
        loop {
            select!(
                msg = ser::Msg::receive(&mut reader) => self.process_msg(msg.with_context(|| "reading a message from server failed")?).await,
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
                f.save(&self.file_dir).await.unwrap_or_else(|e| {
                    eprintln!("...saving the file \"{:?}\" failed! Err: {:?}", f.name(), e)
                });
            }
            ser::Msg::DataFrom {
                data: Data::Image(image),
                from,
            } => {
                println!("Received image from {from}...");
                match if self.save_png {
                    image.save_as_png(&self.img_dir).await
                } else {
                    image.save(&self.img_dir).await
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
            ser::Msg::Error(ser::Error::NotAuthenticated(msg)) => {
                eprintln!("You need to .login or .signup before sending a message (parsed message: {msg})")
            }
            ser::Msg::Error(ser::Error::AlreadyAuthenticated) => {
                eprintln!("You are currently logged in, if you want to log in as another user first log out.")
            }
            ser::Msg::Error(err) => eprintln!("Error: {err:?}"),
        };
    }

    /// Parses incoming lines into messages and writes them to the writer.
    ///
    /// When lines are closed, sends a quit signal to the one-shot channel.
    async fn handle_input<S>(
        mut lines: mpsc::Receiver<String>,
        mut writer: S,
        quit: oneshot::Sender<()>,
    ) -> anyhow::Result<()>
    where
        S: AsyncWriteExt + std::marker::Unpin + std::marker::Send,
    {
        while let Some(line) = lines.recv().await {
            match parse_msg(&line).await {
                Ok(msg) => msg
                    .send(&mut writer)
                    .await
                    .with_context(|| "sending your message to the server failed")?,
                Err(e) => eprintln!("Couldn't create your message (error: {e:?})"),
            };
        }
        quit.send(())
            .map_err(|_| anyhow!("Sending a quit signal to the message receiver failed"))?;
        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq)]
struct NotQuit;
#[derive(Debug, PartialEq, Eq)]
struct Quit;
impl FromStr for Quit {
    type Err = NotQuit;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if Regex::new(r"^\s*\.quit\s*$").unwrap().is_match(s) {
            Ok(Quit)
        } else {
            Err(NotQuit)
        }
    }
}

/// Parses message from string, files and images are read into memory.
///
/// TODO: Document special phrases and its usage.
/// TODO: Handle wrong usage such as .login without less than two arguments.
async fn parse_msg(line: &str) -> anyhow::Result<cli::Msg> {
    let err_msg = "unexpected regex error, contact the crate implementer";
    let reg_file = Regex::new(r"^\s*\.file\s+(?<file>\S+)\s*.*$").expect(err_msg);
    let reg_image = Regex::new(r"^\s*\.image\s+(?<image>\S+)\s*.*$").expect(err_msg);
    let reg_log_in = Regex::new(r"^\s*\.login\s+(?<name>\S+)\s+(?<pswd>\S+)\s*.*$").expect(err_msg);
    let reg_sign_up =
        Regex::new(r"^\s*\.signup\s+(?<name>\S+)\s+(?<pswd>\S+)\s*.*$").expect(err_msg);

    let msg = if let Some((_, [file])) = reg_file.captures(line).map(|caps| caps.extract()) {
        cli::Msg::ToAll(File::from_path(file).await?.into())
    } else if let Some((_, [image])) = reg_image.captures(line).map(|caps| caps.extract()) {
        cli::Msg::ToAll(Image::from_path(image).await?.into())
    } else if let Some((_, [name, pswd])) = reg_log_in.captures(line).map(|caps| caps.extract()) {
        cli::Msg::Auth(cli::Auth::LogIn(cli::Credentials {
            user: name.to_string().into(),
            password: pswd.to_string(),
        }))
    } else if let Some((_, [name, pswd])) = reg_sign_up.captures(line).map(|caps| caps.extract()) {
        cli::Msg::Auth(cli::Auth::SignUp(cli::Credentials {
            user: name.to_string().into(),
            password: pswd.to_string(),
        }))
    } else {
        cli::Msg::ToAll(Data::Text(line.to_string()))
    };
    Ok(msg)
}

/// TODO: more tests for the parse_msg.
#[cfg(test)]
mod tests {
    use super::*;
    use core::time::Duration;

    #[tokio::test]
    async fn parse_quit() {
        assert_eq!(Quit, ".quit".parse::<Quit>().unwrap());
        assert_eq!(Quit, "      .quit      ".parse::<Quit>().unwrap());
        assert_eq!(NotQuit, "      text      ".parse::<Quit>().unwrap_err());
    }

    #[tokio::test]
    async fn parse_msg_file() {
        let path = "Cargo.toml";
        assert_eq!(
            cli::Msg::ToAll(File::from_path(path).await.unwrap().into()),
            parse_msg(&*format!(".file {path}")).await.unwrap()
        );
    }

    #[tokio::test]
    async fn parse_msg_img() {
        let path = "../example-images/rustacean-orig-noshadow.png";
        assert_eq!(
            cli::Msg::ToAll(Image::from_path(path).await.unwrap().into()),
            parse_msg(&*format!(".image {path}")).await.unwrap()
        );
    }

    #[tokio::test]
    async fn parse_msg_text() {
        for s in ["some text"] {
            assert_eq!(
                cli::Msg::ToAll(Data::Text(s.to_string())),
                parse_msg(s).await.unwrap()
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
