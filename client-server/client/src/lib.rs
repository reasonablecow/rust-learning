//! # Client Executable
//!
//! Built on top of [cli-ser](https://github.com/reasonablecow/rust-learning/tree/hw-lesson-16/client-server/cli-ser) crate.
//!
//! Meant to be run as a command line application. For options see:
//! ```sh
//! cargo run -- --help
//! ```
//!
//! ## User Input Commands
//!
//! When interacting with the running client, input beginning with a dot is interpreted as a command. Here is the list of available commands:
//!
//! * `.signup <USER> <PASSWORD>` - sends request to create the user.
//! * `.login <USER> <PASSWORD>` - sends a request to log in with the user.
//! * `.file <PATH>` - tries to load and send the file.
//! * `.image <PATH>` - tries to load and send the image.
//! * `.quit` - tells the application to shut down.
//!
//! Any text without a leading dot is transmitted as a **text** message.
// TODO: Add ".help" or similar to see how to make messages right from the client.
// TODO: Make CMD_PREFIX configurable by the user.
use std::{net::SocketAddr, path::PathBuf, str::FromStr};

use anyhow::{anyhow, Context};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    select,
    sync::{mpsc, oneshot},
};

use cli_ser::{cli, ser, Data, File, Image, Messageable};

/// Default server host.
pub const HOST_DEFAULT: [u8; 4] = [127, 0, 0, 1];
/// Default server port.
pub const PORT_DEFAULT: u16 = 11111;

/// Client configurations.
// Idea: maybe implement std Default for this...
#[derive(Clone)]
pub struct Config {
    /// Path to save received files.
    pub file_dir: PathBuf,
    /// Path to save received images.
    pub img_dir: PathBuf,
    /// Address of the server to connect to.
    pub addr: SocketAddr,
    /// Whether to save all images as PNGs.
    pub save_png: bool,
}

/// Connects to the server, sends messages (read form the terminal) to it, and prints received ones.
///
/// Spawns stdin parser thread, tcp sender and tcp receiver tasks.
///
/// For input commands see [client][self].
pub async fn run(config: Config) -> anyhow::Result<()> {
    let (reader, writer) = TcpStream::connect(config.addr)
        .await
        .with_context(|| {
            "Connection to the server failed, please make sure the server is running."
        })?
        .into_split();
    // Channel to indicate to stop receiving for messages.
    let (quit_sender, quit_receiver) = oneshot::channel();
    // Channel to pass input read in blocking thread to the async handle task.
    let (input_producer, input_consumer) = mpsc::channel(128);

    let stdin_parser = std::thread::spawn(move || parse_stdin(input_producer));
    let msg_receiver = tokio::spawn(receive_in_loop(config.clone(), reader, quit_receiver));
    let msg_sender = tokio::spawn(handle_input(input_consumer, writer, quit_sender));

    // Awaiting the msg_receiver first is important for crash to show up when it happens.
    msg_receiver
        .await?
        .with_context(|| "Receiver went through an unrecoverable error")?;
    msg_sender
        .await?
        .with_context(|| "Message sender crashed.")?;

    // thread .join()'s Err variant does not implement Error trait -> .expect.
    stdin_parser
        .join()
        .expect("stdin parser thread should never panic")
        .with_context(|| "standard input parser crashed.")?;
    Ok(())
}

/// Reads lines from standard input, parses them and sends the result over the `sender` channel until a [Quit][Command::Quit] is parsed.
// The practice of spawning a blocking thread for interactive user input, is advised in
// the [tokio documentation](https://docs.rs/tokio_wasi/latest/tokio/io/fn.stdin.html).
//
// I couldn't find an example of such an implementation.
// [Google Comprehensive Rust - Chat Application](https://google.github.io/comprehensive-rust/exercises/concurrency/chat-app.html)
// uses BufReader, however it results in a need of at least one "enter" stroke for tests.
//
// The behavior is documented here
// ["For technical reasons, stdin is implemented by using an ordinary blocking read
// on a separate thread, and it is impossible to cancel that read.
// This can make shutdown of the runtime hang until the user presses enter."](https://docs.rs/tokio/latest/tokio/io/struct.Stdin.html)
fn parse_stdin(sender: mpsc::Sender<Result<MsgCmd, ParseInputError>>) -> anyhow::Result<()> {
    for line in std::io::stdin().lines() {
        let line = line.with_context(|| "Reading a line from stdin failed.")?;
        let parsed = match line.parse::<Command>() {
            Ok(Command::Quit) => break,
            Ok(Command::Msg(cmd)) => Ok(cmd),
            Err(e) => Err(e),
        };
        sender
            .blocking_send(parsed)
            .with_context(|| "Sending the parsed input over the channel failed.")?;
    }
    Ok(())
}

/// A command to make a message.
#[derive(Debug, PartialEq)]
enum MsgCmd {
    File(String),
    Image(String),
    LogIn(String, String),
    SignUp(String, String),
    NoCmd(String),
}

#[derive(Debug)]
struct ParseInputError(String);

/// A user command.
#[derive(Debug, PartialEq)]
enum Command {
    Msg(MsgCmd),
    Quit,
}
impl From<MsgCmd> for Command {
    fn from(cmd: MsgCmd) -> Self {
        Self::Msg(cmd)
    }
}
impl FromStr for Command {
    type Err = ParseInputError;

    fn from_str(line: &str) -> Result<Self, Self::Err> {
        let mut words = line.split_whitespace();
        match words.next().and_then(|s| s.strip_prefix('.')) {
            Some("quit") => match words.next() {
                None => Ok(Self::Quit),
                Some(_) => Err(ParseInputError(
                    ".quit command can not be followed by any text!".to_string(),
                )),
            },
            Some("file") => match (words.next(), words.next()) {
                (Some(file), None) => Ok(MsgCmd::File(file.to_string()).into()),
                _ => Err(ParseInputError(
                    "command \".file\" requires a path as the only argument!".to_string(),
                )),
            },
            Some("image") => match (words.next(), words.next()) {
                (Some(path), None) => Ok(MsgCmd::Image(path.to_string()).into()),
                _ => Err(ParseInputError(
                    "command \".image\" requires the path as the only argument!".to_string(),
                )),
            },
            Some("login") => match (words.next(), words.next(), words.next()) {
                (Some(name), Some(pswd), None) => {
                    Ok(MsgCmd::LogIn(name.to_string(), pswd.to_string()).into())
                }
                _ => Err(ParseInputError(
                    "command \".login\" needs a username, password and nothing else!".to_string(),
                )),
            },
            Some("signup") => match (words.next(), words.next(), words.next()) {
                (Some(name), Some(pswd), None) => {
                    Ok(MsgCmd::SignUp(name.to_string(), pswd.to_string()).into())
                }
                _ => Err(ParseInputError(
                    "command \".signup\" needs a username, password and nothing else!".to_string(),
                )),
            },
            Some(cmd) => Err(ParseInputError(format!(
                "command \".{cmd}\" is not supported"
            ))),
            _ => Ok(MsgCmd::NoCmd(line.to_string()).into()),
        }
    }
}

/// Receives and processes messages from the server until quit message comes.
async fn receive_in_loop<R>(
    config: Config,
    mut reader: R,
    mut quit: oneshot::Receiver<()>,
) -> anyhow::Result<()>
where
    R: AsyncReadExt + std::marker::Unpin + std::marker::Send,
{
    loop {
        select!(
            msg = ser::Msg::receive(&mut reader) => process_msg(&config, msg.with_context(|| "reading a message from server failed")?).await,
            _ = &mut quit => break Ok(()),
        )
    }
}

/// Processes the message, depending on the type, it either prints it or writes it to a file.
async fn process_msg(config: &Config, msg: ser::Msg) {
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
            f.save(&config.file_dir).await.unwrap_or_else(|e| {
                eprintln!("...saving the file \"{:?}\" failed! Err: {:?}", f.name(), e)
            });
        }
        ser::Msg::DataFrom {
            data: Data::Image(image),
            from,
        } => {
            println!("Received image from {from}...");
            match if config.save_png {
                image.save_as_png(&config.img_dir).await
            } else {
                image.save(&config.img_dir).await
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
            eprintln!(
                "You need to .login or .signup before sending a message (parsed message: {msg})"
            )
        }
        ser::Msg::Error(ser::Error::AlreadyAuthenticated) => {
            eprintln!(
                "You are currently logged in, if you want to log in as another user first log out."
            )
        }
        ser::Msg::Error(err) => eprintln!("Error: {err:?}"),
    };
}

/// Makes messages from incoming parsed input, when successful, writes them to the `writer`.
///
/// When `inputs` are closed, sends a quit signal to the `quit` one-shot channel.
async fn handle_input<W>(
    mut inputs: mpsc::Receiver<Result<MsgCmd, ParseInputError>>,
    mut writer: W,
    quit: oneshot::Sender<()>,
) -> anyhow::Result<()>
where
    W: AsyncWriteExt + std::marker::Unpin + std::marker::Send,
{
    while let Some(input) = inputs.recv().await {
        match input {
            Err(e) => {
                eprintln!("Couldn't parse your command! {e:?}");
            }
            Ok(cmd) => match make_message(cmd).await {
                Ok(msg) => msg
                    .send(&mut writer)
                    .await
                    .with_context(|| "sending your message to the server failed")?,
                Err(e) => eprintln!("Couldn't make your message! {e:?}"),
            },
        }
    }
    quit.send(())
        .map_err(|_| anyhow!("Sending a quit signal to the message receiver failed"))?;
    Ok(())
}

/// Makes a message from the [MsgCmd].
async fn make_message(command: MsgCmd) -> anyhow::Result<cli::Msg> {
    let msg = match command {
        MsgCmd::File(path) => cli::Msg::ToAll(File::from_path(path).await?.into()),
        MsgCmd::Image(path) => cli::Msg::ToAll(Image::from_path(path).await?.into()),
        MsgCmd::LogIn(username, password) => cli::Msg::Auth(cli::Auth::LogIn(cli::Credentials {
            user: username.to_string().into(),
            password: password.to_string(),
        })),
        MsgCmd::SignUp(username, password) => cli::Msg::Auth(cli::Auth::SignUp(cli::Credentials {
            user: username.to_string().into(),
            password: password.to_string(),
        })),
        MsgCmd::NoCmd(text) => cli::Msg::ToAll(Data::Text(text)),
    };
    Ok(msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cmd_quit() {
        assert_eq!(Command::Quit, ".quit".parse::<Command>().unwrap());
        assert_eq!(
            Command::Quit,
            "      .quit      ".parse::<Command>().unwrap()
        );
        assert!("  .quit    text      ".parse::<Command>().is_err());
    }

    #[test]
    fn parse_cmd_file() {
        let path = "Cargo.toml";
        assert_eq!(
            format!(".file {path}").parse::<Command>().unwrap(),
            Command::Msg(MsgCmd::File(path.to_string()))
        );
        assert!(".file           ".parse::<Command>().is_err());
        assert!("    .file   one   two".parse::<Command>().is_err());
    }

    #[test]
    fn parse_cmd_image() {
        let path = "../example-images/rustacean-orig-noshadow.png";
        assert_eq!(
            format!(".image {path}").parse::<Command>().unwrap(),
            Command::Msg(MsgCmd::Image(path.to_string()))
        );
        assert!(".image  ".parse::<Command>().is_err());
        assert!("    .image   foo   bar".parse::<Command>().is_err());
    }

    #[test]
    fn parse_login() {
        assert!("    .login  ".parse::<Command>().is_err());
        assert!("    .login  name  ".parse::<Command>().is_err());
        assert!(".login  name password details".parse::<Command>().is_err());
        let (name, password) = ("test_name", "test_pswd");
        assert_eq!(
            format!(".login {name} {password}")
                .parse::<Command>()
                .unwrap(),
            Command::Msg(MsgCmd::LogIn(name.to_string(), password.to_string()))
        );
    }

    #[test]
    fn parse_signup() {
        assert!(".signup  ".parse::<Command>().is_err());
        assert!(".signup first  ".parse::<Command>().is_err());
        assert!("  .signup first second third".parse::<Command>().is_err());
        let (name, password) = ("some-name", "3l1k54j2l6kj23");
        assert_eq!(
            format!(".signup {name} {password}")
                .parse::<Command>()
                .unwrap(),
            Command::Msg(MsgCmd::SignUp(name.to_string(), password.to_string()))
        );
    }

    #[test]
    fn parse_unknown() {
        assert!("    .exit  ".parse::<Command>().is_err());
        assert!(".bye  ".parse::<Command>().is_err());
        assert!("   .hello".parse::<Command>().is_err());
        assert!("...all good".parse::<Command>().is_err());
    }

    #[test]
    fn parse_no_cmd() {
        for s in ["some text", "            ", "bye.quit"] {
            assert_eq!(
                s.parse::<Command>().unwrap(),
                Command::Msg(MsgCmd::NoCmd(s.to_string()))
            );
        }
    }
}
