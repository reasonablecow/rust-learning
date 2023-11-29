//! Client-Server Utilities

use std::{
    error,
    ffi::{OsStr, OsString},
    fs,
    io::{self, Cursor, ErrorKind, Read, Write},
    net::TcpStream,
    path::{Path, PathBuf},
    result,
};

use chrono::{offset::Utc, SecondsFormat};
use image::ImageFormat;
use serde::{Deserialize, Serialize};

use crate::Error::*;

type Result<T> = result::Result<T, Error>;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("setting TCP stream to a nonblocking mode (to check for a receivable bytes) failed")]
    SetNonblocking(io::Error),
    #[error("setting TCP stream to a blocking mode (to read all bytes together) failed")]
    SetBlocking(io::Error),
    #[error("receiving byte count from the stream failed")]
    ReceiveByteCnt(io::Error),
    #[error("receiving bytes from the stream failed")]
    ReceiveBytes(io::Error),
    #[error("sending byte count over the stream failed")]
    SendByteCnt(io::Error),
    #[error("sending bytes over the stream failed")]
    SendBytes(io::Error),
    #[error("flushing the stream failed")]
    FlushStream(io::Error),
    #[error("serialization of the message failed")]
    SerializeMsg(bincode::Error),
    #[error("deserialization of the message failed")]
    DeserializeMsg(bincode::Error),
}

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
    pub fn save(&self, dir: &Path) -> result::Result<PathBuf, Box<dyn error::Error>> {
        let path = Self::create_path(dir, self.format);
        fs::File::create(&path)? // File creation failed.
            .write_all(&self.bytes)?; // Writing the file failed.
        Ok(path)
    }

    pub fn save_as_png(self, dir: &Path) -> result::Result<PathBuf, Box<dyn error::Error>> {
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
    type Error = Box<dyn error::Error>;

    fn try_from(value: &Path) -> result::Result<Self, Self::Error> {
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
    pub fn save(&self, path: impl AsRef<Path>) -> io::Result<()> {
        fs::File::create(path.as_ref().join(&self.name))? // File creation failed.
            .write_all(&self.bytes)?; // Writing the file failed.
        Ok(())
    }
}
impl TryFrom<&Path> for File {
    type Error = Box<dyn error::Error>;

    fn try_from(value: &Path) -> result::Result<Self, Self::Error> {
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
    pub fn file_from_path(path: impl AsRef<Path>) -> result::Result<Self, Box<dyn error::Error>> {
        Ok(File::try_from(path.as_ref())?.into())
    }

    pub fn img_from_path(path: impl AsRef<Path>) -> result::Result<Self, Box<dyn error::Error>> {
        Ok(Image::try_from(path.as_ref())?.into())
    }

    /// Serializes Message into bytes.
    pub fn serialize(&self) -> Result<Vec<u8>> {
        bincode::serialize(self).map_err(SerializeMsg)
    }

    /// Deserialize Message from bytes.
    pub fn deserialize(bytes: &[u8]) -> Result<Self> {
        bincode::deserialize(bytes).map_err(DeserializeMsg)
    }

    pub fn receive(stream: &mut TcpStream) -> Result<Option<Self>> {
        receive_bytes(stream)?
            .as_deref()
            .map(Self::deserialize)
            .transpose()
    }

    pub fn send(&self, stream: &mut TcpStream) -> Result<()> {
        self.serialize()
            .and_then(|bytes| send_bytes(stream, &bytes))
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

/// Tries to receive bytes (first asks for byte count) from given stream.
///
/// Asks for the byte count in a non-blocking manner; upon success, the byte reading becomes blocking.
pub fn receive_bytes(stream: &mut TcpStream) -> Result<Option<Vec<u8>>> {
    stream.set_nonblocking(true).map_err(SetNonblocking)?;
    let mut len_bytes = [0u8; 4];
    match stream.read_exact(&mut len_bytes) {
        Ok(()) => {
            stream.set_nonblocking(false).map_err(SetBlocking)?;
            let len = u32::from_be_bytes(len_bytes) as usize;
            let mut bytes = vec![0u8; len];
            stream.read_exact(&mut bytes).map_err(ReceiveBytes)?;
            Ok(Some(bytes))
        }
        Err(e) => match e.kind() {
            ErrorKind::WouldBlock // No message is ready
            | ErrorKind::UnexpectedEof // The stream was closed
            => Ok(None),
            _ => Err(ReceiveByteCnt(e)),
        },
    }
}

/// Tries to send bytes over given stream.
pub fn send_bytes(stream: &mut TcpStream, bytes: &[u8]) -> Result<()> {
    stream
        .write_all(&((bytes.len() as u32).to_be_bytes()))
        .map_err(SendByteCnt)?;
    stream.write_all(bytes).map_err(SendBytes)?;
    stream.flush().map_err(FlushStream)?;
    Ok(())
}
