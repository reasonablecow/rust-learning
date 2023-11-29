//! Client-Server Utilities
//!
//! TODO: cli-ser error enum instead of all panics

use std::{
    fs,
    io::{self, Cursor, ErrorKind, Read, Write},
    net::TcpStream,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

use chrono::{offset::Utc, SecondsFormat};
use image::ImageFormat;
use serde::{Deserialize, Serialize};

/// Remote definition of image::ImageFormat for de/serialization.
/// Based on <https://serde.rs/remote-derive.html>.
#[derive(Serialize, Deserialize, Debug, Copy, Clone, PartialEq)]
#[serde(remote = "ImageFormat")]
#[non_exhaustive]
pub enum ImageFormatDef {
    Png,
    Jpeg,
    Gif,
    WebP,
    Pnm,
    Tiff,
    Tga,
    Dds,
    Bmp,
    Ico,
    Hdr,
    OpenExr,
    Farbfeld,
    Avif,
    Qoi,
}

#[derive(Serialize, Deserialize, PartialEq, Debug)]
pub struct Image {
    #[serde(with = "ImageFormatDef")]
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
            image::io::Reader::with_format(Cursor::new(self.bytes), self.format)
                .decode()
                .expect("Image decoding failed.")
                .save_with_format(Self::create_path(dir, ImageFormat::Png), ImageFormat::Png)
                .expect("Encoding and writing to a file should work.");
        } else {
            self.save(dir);
        }
    }

    fn create_path(dir: &Path, format: ImageFormat) -> PathBuf {
        dir.join(format!(
            "{}.{}",
            Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
            // It's safe: <https://docs.rs/image/latest/src/image/image.rs.html#290-309>.
            format.extensions_str()[0]
        ))
    }
}

#[derive(Serialize, Deserialize, PartialEq, Debug)]
pub struct File {
    // TODO: make the fields private with getters for future stability
    pub name: PathBuf,
    pub bytes: Vec<u8>,
}

/// Messages to be sent over the network.
#[derive(Serialize, Deserialize, PartialEq, Debug)]
pub enum Message {
    Text(String),
    File(File),
    Image(Image),
}
impl Message {
    pub fn file_from_path(path: String) -> Message {
        let path = PathBuf::from(path);
        let name = path
            .file_name()
            .expect("Path given does not end with a valid file name.")
            .into();
        let mut file = fs::File::open(path).expect("File for the given path can not be opened.");
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .expect("Reading the specified file failed.");
        Message::File(File { name, bytes })
    }

    pub fn img_from_path(path: String) -> Message {
        let reader = image::io::Reader::open(path)
            .expect("Image opening failed.")
            .with_guessed_format()
            .expect("The format should be deducible.");

        let format = reader.format().expect("The image format must be clear!");

        let mut bytes = Vec::new();
        reader
            .into_inner()
            .read_to_end(&mut bytes)
            .expect("Reading the specified file failed.");

        Message::Image(Image { format, bytes })
    }
}

/// Tries to read a message in a nonblocking fashion.
///
/// Panics for other io::Errors than empty or closed streams.
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
/// Panics when serialization fails (should never happen).
pub fn serialize_msg(msg: &Message) -> Vec<u8> {
    bincode::serialize(msg)
        .expect("Message serialization should always work - contact the implementer!")
}

/// Sends bytes over given stream.
///
/// Panics! BrokenPipe error kind occurs when sending a message to a closed stream.
pub fn send_bytes(stream: &mut TcpStream, bytes: &Vec<u8>) -> Result<(), io::Error> {
    stream.write_all(&((bytes.len() as u32).to_be_bytes()))?;
    stream.write_all(bytes)?;
    stream.flush()?;
    Ok(())
}

/// Deprecated! Useful for testing initially.
/// TODO: Add a test simulating simple client and server.
pub fn _simulate_connections() {
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
