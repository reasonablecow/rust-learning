//! # Client Executable
//!
//! See:
//! ```sh
//! cargo run -- --help
//! ```
use std::{
    error::Error,
    fs,
    io::{self, Write},
    net::TcpStream,
    path::Path,
    sync::mpsc,
    thread,
    time::Duration,
};

use clap::Parser;
use regex::Regex;

use cli_ser::{read_msg, send_bytes, serialize_msg, Message};

/* // Dunno how to do lazy statics...
use once_cell::sync::Lazy;
static FILES_DIR: Lazy<PathBuf> = Lazy::new(|| PathBuf::from("files"));
*/

/// Client executable, interactively sends messages to the specified server.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Server host
    #[arg(long, default_value_t = String::from("127.0.0.1"))]
    host: String,

    /// Server port
    #[arg(short, long, default_value_t = 11111)]
    port: u32,

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
    let (end_sender, end_receiver) = mpsc::channel();

    // Reads messages from the server in the background.
    let receiver = thread::spawn(move || loop {
        if let Some(msg) = read_msg(&mut stream_clone) {
            match msg {
                Message::Text(text) => println!("{}", text),
                Message::File(f) => {
                    println!("Received {:?}", f.name);
                    let path = files_dir.join(f.name);
                    fs::File::create(path)
                        .expect("File creation failed.")
                        .write_all(&f.bytes)
                        .expect("Writing the file failed.");
                }
                Message::Image(image) => {
                    if args.save_png {
                        image.save_as_png(images_dir);
                    } else {
                        image.save(images_dir);
                    }
                    println!("Received image...");
                }
            }
        } else if end_receiver.try_recv().is_ok() {
            break;
        }
    });

    // Parses and executes commands given by the user.
    loop {
        println!("Please type the command:");
        let msg = match Command::from_stdin() {
            Command::Quit => {
                println!("Goodbye!");
                // Time for messages which were sent but were not receivable yet.
                thread::sleep(Duration::from_secs(5));
                end_sender
                    .send(true)
                    .expect("Program crashed during closing.");
                break;
            }
            cmd => Message::try_from(cmd).expect("User provided wrong command."),
        };
        send_bytes(&mut stream, &serialize_msg(&msg))
            .expect("Sending of you message failed, please restart and try again.");
    }

    // Wait for the receiver to finish its job.
    receiver
        .join()
        .expect("Message receiver crashed, sorry for the inconvenience.");
}

/// Commands useful for the client user.
#[derive(Debug, PartialEq)]
pub enum Command {
    Quit,
    File(String),
    Image(String),
    Other(String),
}

impl Command {
    pub fn from_stdin() -> Command {
        let mut line = String::new();
        io::stdin()
            .read_line(&mut line)
            .expect("reading standard input should work.");
        let line = if let Some(stripped) = line.strip_suffix('\n') {
            stripped.to_string()
        } else {
            line
        };
        Command::from_str(&line)
    }

    fn from_str(s: &str) -> Command {
        let err_msg = "unexpected regex error, contact the crate implementer";
        let reg_quit = Regex::new(r"^\s*\.quit\s*$").expect(err_msg);
        let reg_file = Regex::new(r"^\s*\.file\s+(?<file>\S+.*)\s*$").expect(err_msg);
        let reg_image = Regex::new(r"^\s*\.image\s+(?<image>\S+.*)\s*$").expect(err_msg);

        if reg_quit.is_match(s) {
            Command::Quit
        } else if let Some((_, [file])) = reg_file.captures(s).map(|caps| caps.extract()) {
            Command::File(file.to_string())
        } else if let Some((_, [image])) = reg_image.captures(s).map(|caps| caps.extract()) {
            Command::Image(image.to_string())
        } else {
            Command::Other(s.to_string())
        }
    }
}

impl TryFrom<Command> for Message {
    type Error = Box<dyn Error>;

    fn try_from(value: Command) -> Result<Self, Self::Error> {
        match value {
            Command::Quit => Err("A Massage can not be constructed from a Quit command!".into()),
            Command::Other(s) => Ok(Message::Text(s)),
            Command::File(path) => Ok(Message::file_from_path(path)),
            Command::Image(path) => Ok(Message::img_from_path(path)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_from_str_quit() {
        assert_eq!(Command::Quit, Command::from_str(".quit"));
        assert_eq!(Command::Quit, Command::from_str("      .quit      "));
    }

    #[test]
    fn command_from_str_file() {
        let f = "a.txt";
        assert_eq!(
            Command::File(String::from(f)),
            Command::from_str(&format!(".file {}", f))
        );
    }

    #[test]
    fn command_from_str_image() {
        let i = "i.png";
        assert_eq!(
            Command::Image(String::from(i)),
            Command::from_str(&format!(".image {}", i))
        );
    }

    #[test]
    fn command_from_str_other() {
        for s in [". quit", ".quit      s", "a   .quit "] {
            assert_eq!(Command::Other(String::from(s)), Command::from_str(s));
        }
    }

    #[test]
    fn test_run() {
        let listener = std::net::TcpListener::bind("127.0.0.1:11111")
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
