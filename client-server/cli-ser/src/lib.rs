//! Client-Server Utilities
//!
//! TODO: cli-ser error enum instead of all panics

use std::{
    error::Error,
    ffi::{OsStr, OsString},
    fs,
    io::{self, Cursor, ErrorKind, Read, Write},
    net::TcpStream,
    path::{Path, PathBuf},
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
    pub fn save(&self, dir: &Path) -> Result<PathBuf, Box<dyn Error>> {
        let path = Self::create_path(dir, self.format);
        fs::File::create(&path)? // File creation failed.
            .write_all(&self.bytes)?; // Writing the file failed.
        Ok(path)
    }

    pub fn save_as_png(self, dir: &Path) -> Result<PathBuf, Box<dyn Error>> {
        if self.format != ImageFormat::Png {
            let path = Self::create_path(dir, ImageFormat::Png);
            image::io::Reader::with_format(Cursor::new(self.bytes), self.format)
                .decode()? // Image decoding failed.
                // Encoding and writing to a file should work.
                .save_with_format(&path, ImageFormat::Png)?;
            Ok(path)
        } else {
            self.save(dir)
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
impl TryFrom<&Path> for Image {
    type Error = Box<dyn Error>;

    fn try_from(value: &Path) -> Result<Self, Self::Error> {
        let reader = image::io::Reader::open(value)? // Image opening failed.
            .with_guessed_format()?;
        let format = reader
            .format()
            .ok_or_else(|| format!("Image image format of \"{:?}\" wasn't recognized.", value))?;

        let mut bytes = Vec::new();
        reader.into_inner().read_to_end(&mut bytes)?; // Reading the specified file failed.
        Ok(Image { format, bytes })
    }
}

#[derive(Serialize, Deserialize, PartialEq, Debug)]
pub struct File {
    name: OsString,
    bytes: Vec<u8>,
}
impl File {
    pub fn name(&self) -> &OsStr {
        &self.name
    }
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), io::Error> {
        fs::File::create(path.as_ref().join(&self.name))? // File creation failed.
            .write_all(&self.bytes)?; // Writing the file failed.
        Ok(())
    }
}
impl TryFrom<&Path> for File {
    type Error = Box<dyn Error>;

    fn try_from(value: &Path) -> Result<Self, Self::Error> {
        let name = value
            .file_name()
            .ok_or("Path given does not end with a valid file name.")?
            .to_os_string();
        let mut file = fs::File::open(value)?; // File for the given path can not be opened.
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?; //Reading the specified file failed.
        Ok(File { name, bytes })
    }
}

/// Messages to be sent over the network.
#[derive(Serialize, Deserialize, PartialEq, Debug)]
pub enum Message {
    Text(String),
    File(File),
    Image(Image),
}
impl Message {
    pub fn file_from_path(path: impl AsRef<Path>) -> Result<Message, Box<dyn Error>> {
        Ok(File::try_from(path.as_ref())?.into())
    }

    pub fn img_from_path(path: impl AsRef<Path>) -> Result<Message, Box<dyn Error>> {
        Ok(Image::try_from(path.as_ref())?.into())
    }
}
impl From<File> for Message {
    fn from(value: File) -> Message {
        Message::File(value)
    }
}
impl From<Image> for Message {
    fn from(value: Image) -> Message {
        Message::Image(value)
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
pub fn send_bytes(stream: &mut TcpStream, bytes: &Vec<u8>) -> Result<(), io::Error> {
    stream.write_all(&((bytes.len() as u32).to_be_bytes()))?;
    stream.write_all(bytes)?;
    stream.flush()?;
    Ok(())
}
