//! Client Server Utilities
//!
//! ## Examples
//! *
//! * https://github.com/rust-community/resources/blob/gh-pages/sticker/rust/examples/hexagon.jpeg

use std::{
    error::Error,
    fs,
    io::{self, Cursor, ErrorKind, Read, Write},
    net::TcpStream,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

use chrono::{offset::Utc, SecondsFormat};
use regex::Regex;
use serde::{Deserialize, Serialize};

/// This whole thing wouldn't exist if image::ImageFormat would be serializable
#[derive(Serialize, Deserialize, Debug, Copy, Clone, PartialEq)]
pub enum ImageFormat {
    Png,
    Jpeg,
    //...todo
}
impl ImageFormat {
    fn to_str(self) -> &'static str {
        // Every format has at least one <https://docs.rs/image/latest/src/image/image.rs.html#290-309>.
        self.to_official().extensions_str()[0]
    }

    fn to_official(self) -> image::ImageFormat {
        match self {
            ImageFormat::Png => image::ImageFormat::Png,
            ImageFormat::Jpeg => image::ImageFormat::Jpeg,
        }
    }

    fn from_official(format: image::ImageFormat) -> Self {
        match format {
            image::ImageFormat::Png => ImageFormat::Png,
            image::ImageFormat::Jpeg => ImageFormat::Jpeg,
            _ => todo!(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Image {
    format: ImageFormat,
    bytes: Vec<u8>,
}

impl Image {
    pub fn save(&self, dir: &Path) {
        fs::File::create(Self::create_path(dir, self.format))
            .expect("File creation failed.")
            .write_all(&self.bytes)
            .expect("Writing the file failed.");
    }

    pub fn save_as_png(self, dir: &Path) {
        if self.format != ImageFormat::Png {
            image::io::Reader::with_format(Cursor::new(self.bytes), self.format.to_official())
                .decode()
                .expect("Image decoding failed.")
                .save_with_format(
                    Self::create_path(dir, ImageFormat::Png),
                    image::ImageFormat::Png,
                )
                .expect("Encoding and writing to a file should work.");
        } else {
            self.save(dir);
        }
    }

    fn create_path(dir: &Path, format: ImageFormat) -> PathBuf {
        dir.join(format!(
            "{}.{}",
            Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
            format.to_str()
        ))
    }
}

// TODO private fields
#[derive(Serialize, Deserialize, Debug)]
pub struct File {
    pub name: PathBuf,
    pub bytes: Vec<u8>,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum Message {
    Text(String),
    File(File),
    Image(Image),
}

/// TODO
impl Message {
    pub fn from_cmd(cmd: Command) -> Result<Message, Box<dyn Error>> {
        match cmd {
            Command::Quit => Err("A Massage can not be constructed from a Quit command!".into()),
            Command::Other(s) => Ok(Message::Text(s)),
            Command::File(path) => {
                let path = PathBuf::from(path);
                let name = path
                    .file_name()
                    .expect("Path given does not end with a valid file name.")
                    .into();
                let mut file =
                    fs::File::open(path).expect("File for the given path can not be opened.");
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes)
                    .expect("Reading the specified file failed.");

                Ok(Message::File(File { name, bytes }))
            }
            Command::Image(path) => {
                let reader = image::io::Reader::open(path)
                    .expect("Image opening failed.")
                    .with_guessed_format()
                    .expect("The format should be deducible.");

                let format = ImageFormat::from_official(
                    reader.format().expect("The image format must be clear!"),
                );

                let mut bytes = Vec::new();
                reader
                    .into_inner()
                    .read_to_end(&mut bytes)
                    .expect("Reading the specified file failed.");

                Ok(Message::Image(Image { format, bytes }))
            }
        }
    }
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub enum Command {
    Quit,
    File(String),
    Image(String),
    Other(String),
}

impl Command {
    pub fn from_stdin() -> Command {
        println!("Please type the command:");
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

/// Tries to read a message in a nonblocking fashion.
///
/// Panics for other io::Error kinds than WouldBlock.
pub fn read_msg(stream: &mut TcpStream) -> Option<Message> {
    stream
        .set_nonblocking(true)
        .expect("Setting non-blocking stream to check for data to be read failed.");
    let mut len_bytes = [0u8; 4];
    match stream.read_exact(&mut len_bytes) {
        Ok(()) => {
            stream
                .set_nonblocking(false)
                .expect("Setting blocking stream to read the data.");
            let len = u32::from_be_bytes(len_bytes) as usize;
            let mut msg_buf = vec![0u8; len];
            stream
                .read_exact(&mut msg_buf)
                .expect("Reading the whole message should be ok.");
            let msg: Message = bincode::deserialize(&msg_buf[..])
                .expect("Deserialization of the read message should be ok.");
            Some(msg)
        }
        Err(e) => match e.kind() {
            ErrorKind::WouldBlock // No message is ready
            | ErrorKind::UnexpectedEof // The stream was closed
            => None,
            _ => panic!("{:?}", e),
        },
    }
}

/// Serializes Message into bytes.
///
/// !Panics if serialization fails (should never happen).
pub fn serialize_msg(msg: &Message) -> Vec<u8> {
    bincode::serialize(msg)
        .expect("Message serialization should always work - contact the implementer!")
}

/// BrokenPipe error kind occurs when sending a message to a closed stream.
pub fn send_bytes(stream: &mut TcpStream, bytes: &Vec<u8>) -> Result<(), io::Error> {
    stream.write_all(&((bytes.len() as u32).to_be_bytes()))?;
    stream.write_all(bytes)?;
    stream.flush()?;
    Ok(())
}

pub fn simulate_connections() {
    let connection_simulator = thread::spawn(move || {
        let mut streams = Vec::new();
        for sth in ["one", "two", "three", "four", "five"] {
            let mut stream = TcpStream::connect("127.0.0.1:11111")
                .expect("TCP stream connection from another thread should be possible.");
            let bytes = serialize_msg(&Message::Text(sth.to_string()));
            send_bytes(&mut stream, &bytes).expect("sending bytes to the server should work");
            streams.push(stream);
        }
        streams
    });
    let streams = connection_simulator
        .join()
        .expect("the streams should be returned.");
    println!("{:?}", streams);
    thread::sleep(Duration::from_secs(3));
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
}
