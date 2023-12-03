//! # Client Executable
//!
//! See:
//! ```sh
//! cargo run -- --help
//! ```
use std::{error::Error, fs, io, net::TcpStream, path::Path, sync::mpsc, thread, time::Duration};

use clap::Parser;
use regex::Regex;

use cli_ser::{send_bytes, Message};

/* // TODO: lazy statics for paths
use once_cell::sync::Lazy;
static FILES_DIR: Lazy<PathBuf> = Lazy::new(|| PathBuf::from("files"));
*/
const HOST_DEFAULT: &str = "127.0.0.1"; // TODO - representation other than str
const PORT_DEFAULT: u16 = 11111;

/// Client executable, interactively sends messages to the specified server.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Server host
    #[arg(long, default_value_t = String::from(HOST_DEFAULT))]
    host: String,

    /// Server port
    #[arg(short, long, default_value_t = PORT_DEFAULT)]
    port: u16,

    /// Save all images as PNG.
    #[arg(short, long, default_value_t = false)]
    save_png: bool,
}

/// Client's main function.
///
/// The main thread in a loop executes commands given by the user.
/// Another thread is spawned in the background to receive messages from the server.
pub fn run() {
    let files_dir = Path::new("files");
    let images_dir = Path::new("images");

    let args = Args::parse();

    fs::create_dir_all(files_dir).expect("Directory for files couldn't be created.");
    fs::create_dir_all(images_dir).expect("Directory for images couldn't be created.");

    let mut stream = TcpStream::connect(format!("{}:{}", args.host, args.port))
        .expect("Connection to the server failed, please make sure the server is running.");

    let mut stream_clone = stream
        .try_clone()
        .expect("The TcpStream should be cloneable.");

    // Channel to indicate to stop waiting for messages.
    let (send_quit, recv_quit) = mpsc::channel();

    // Reads messages from the server in the background.
    let receiver = thread::spawn(move || loop {
        if let Some(msg) =
            Message::receive(&mut stream_clone).expect("reading bytes should never fail")
        {
            match msg {
                Message::Text(text) => println!("{}", text),
                Message::File(f) => {
                    println!("Received {:?}", f.name());
                    f.save(files_dir).unwrap_or_else(|e| {
                        eprintln!("...saving the file \"{:?}\" failed! Err: {:?}", f.name(), e)
                    });
                }
                Message::Image(image) => {
                    println!("Received image...");
                    match if args.save_png {
                        image.save_as_png(images_dir)
                    } else {
                        image.save(images_dir)
                    } {
                        Ok(path) => println!("...image was saved to {:?}", path),
                        Err(e) => eprintln!("...saving the image failed! Err: {:?}", e),
                    }
                }
                Message::Error(err_text) => eprintln!("received error: \"{}\"", err_text),
            }
        } else if recv_quit.try_recv().is_ok() {
            break;
        }
    });

    // Parses and executes commands given by the user.
    loop {
        println!("Please type the command:");
        let msg = match Command::from(&*read_line_from_stdin()) {
            Command::Quit => {
                println!("Goodbye!");
                // Time for messages which were sent but were not receivable yet.
                thread::sleep(Duration::from_secs(5));
                send_quit.send(()).expect("Program crashed during closing.");
                break;
            }
            cmd => Message::try_from(cmd).expect("User provided wrong command."),
        };
        let bytes = msg
            .serialize()
            .expect("message serialization should always work.");
        send_bytes(&mut stream, &bytes)
            .expect("Sending of you message failed, please restart and try again.");
    }

    // Wait for the receiver to finish its job.
    receiver
        .join()
        .expect("Message receiver crashed, sorry for the inconvenience.");
}

/// Reads line from standard input, strips ending newline if present.
/// TODO - return result
fn read_line_from_stdin() -> String {
    let mut line = String::new();
    io::stdin()
        .read_line(&mut line)
        .expect("reading line to get the command failed");
    if let Some(stripped) = line.strip_suffix('\n') {
        stripped.to_string()
    } else {
        line
    }
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
    type Error = Box<dyn Error>;

    fn try_from(value: Command) -> Result<Self, <cli_ser::Message as TryFrom<Command>>::Error> {
        match value {
            Command::Quit => Err("A Massage can not be constructed from a Quit command!".into()),
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

    #[test]
    fn test_run() {
        let listener = std::net::TcpListener::bind(format!("{}:{}", HOST_DEFAULT, PORT_DEFAULT))
            .expect("TCP listener creation should not fail.");
        let server_thread = thread::spawn(move || {
            std::iter::from_fn(|| listener.incoming().next()).collect::<Vec<_>>()
        });
        let client_thread = thread::spawn(move || {
            run();
        });
        thread::sleep(Duration::from_secs(1));
        assert!(!server_thread.is_finished());
        assert!(!client_thread.is_finished());
    }
}