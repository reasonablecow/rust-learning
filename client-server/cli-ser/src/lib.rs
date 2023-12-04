//! Client-Server Utilities

use std::{
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
    #[error("receiving bytes from the stream failed")]
    ReceiveBytes(io::Error),
    #[error("the stream was disconnected")]
    DisconnectedStream(io::Error),
    #[error("sending bytes over the stream failed")]
    SendBytes(io::Error),
    #[error("message serialization failed")]
    SerializeMsg(bincode::Error),
    #[error("deserialization of the message failed")]
    DeserializeMsg(bincode::Error),
    #[error("saving the file failed")]
    SaveFile(io::Error),
    #[error("saving the image failed")]
    SaveImg(io::Error),
    #[error("operation convert and save image failed")]
    ConvertAndSaveImg(image::error::ImageError),
    #[error("loading image with format guessing failed")]
    LoadImg(io::Error),
    #[error("loading file for a given path failed")]
    LoadFile(io::Error),
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
    /// Creates Image from bytes read at path, guesses the image format based on the data.
    ///
    /// Panics: When <https://docs.rs/image/latest/image/io/struct.Reader.html#method.with_guessed_format>
    /// doesn't fail, but <https://docs.rs/image/latest/image/io/struct.Reader.html#method.format> does.
    /// Based on the documentation it should never happen.
    fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let reader = image::io::Reader::open(path)
            .and_then(|r| r.with_guessed_format())
            .map_err(LoadImg)?;
        let format = reader.format().expect("Bug in the crate \"image \"! This should never fail when the previous step has succeeded.");

        let mut bytes = Vec::new();
        reader
            .into_inner()
            .read_to_end(&mut bytes)
            .map_err(LoadImg)?;
        Ok(Image { format, bytes })
    }

    pub fn save(&self, dir: &Path) -> Result<PathBuf> {
        let path = Self::create_path(dir, self.format);
        create_file_and_write_bytes(&path, &self.bytes)
            .map(|_| path)
            .map_err(SaveImg)
    }

    pub fn save_as_png(self, dir: &Path) -> Result<PathBuf> {
        if self.format != ImageFormat::Png {
            let path = Self::create_path(dir, ImageFormat::Png);
            image::io::Reader::with_format(Cursor::new(self.bytes), self.format)
                .decode()
                .and_then(|d| d.save_with_format(&path, ImageFormat::Png))
                .map(|_| path)
                .map_err(ConvertAndSaveImg)
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

#[derive(Serialize, Deserialize, PartialEq, Debug)]
pub struct File {
    name: OsString,
    bytes: Vec<u8>,
}
impl File {
    fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        let mut bytes = Vec::new();
        fs::File::open(&path)
            .and_then(|mut f| f.read_to_end(&mut bytes))
            .map_err(LoadFile)?;
        let name = path
            .as_ref()
            .file_name()
            .unwrap_or(OsStr::new("unknown"))
            .to_os_string();
        Ok(File { name, bytes })
    }

    pub fn name(&self) -> &OsStr {
        &self.name
    }
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        create_file_and_write_bytes(path.as_ref().join(&self.name), &self.bytes).map_err(SaveFile)
    }
}

fn create_file_and_write_bytes(path: impl AsRef<Path>, bytes: &[u8]) -> io::Result<()> {
    fs::File::create(path)?.write_all(bytes)
}

/// Messages to be sent over the network.
///
/// TODO: Client can send Error message to the server which is confusing.
#[derive(Serialize, Deserialize, Debug)]
pub enum Message {
    Text(String),
    File(File),
    Image(Image),
    Error(String),
}
impl Message {
    /// Loads File from a path.
    pub fn file_from_path(path: impl AsRef<Path>) -> Result<Self> {
        File::from_path(path).map(Message::from)
    }

    /// Loads Image from a path.
    pub fn img_from_path(path: impl AsRef<Path>) -> Result<Self> {
        Image::from_path(path).map(Message::from)
    }

    /// Serializes Message into bytes.
    pub fn serialize(&self) -> Result<Vec<u8>> {
        bincode::serialize(self).map_err(SerializeMsg)
    }

    /// Deserialize Message from bytes.
    pub fn deserialize(bytes: &[u8]) -> Result<Self> {
        bincode::deserialize(bytes).map_err(DeserializeMsg)
    }

    /// Tries to receive a message from the given stream.
    pub fn receive(stream: &mut TcpStream) -> Result<Option<Self>> {
        receive_bytes(stream)?
            .as_deref()
            .map(Self::deserialize)
            .transpose()
    }

    /// Sends a message over the given stream.
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
    stream.set_nonblocking(true).map_err(ReceiveBytes)?;
    let mut len_bytes = [0u8; 4];
    match stream.read_exact(&mut len_bytes) {
        Ok(()) => stream
            .set_nonblocking(false)
            .and_then(|_| {
                let len = u32::from_be_bytes(len_bytes) as usize;
                let mut bytes = vec![0u8; len];
                stream.read_exact(&mut bytes).map(|_| Some(bytes))
            })
            .map_err(ReceiveBytes),
        Err(e) => match e.kind() {
            ErrorKind::WouldBlock => Ok(None), // No message is ready
            ErrorKind::UnexpectedEof => Err(DisconnectedStream(e)),
            _ => Err(ReceiveBytes(e)),
        },
    }
}

/// Tries to send bytes over given stream.
pub fn send_bytes(stream: &mut TcpStream, bytes: &[u8]) -> Result<()> {
    stream
        .write_all(&((bytes.len() as u32).to_be_bytes()))
        .and_then(|_| stream.write_all(bytes))
        .and_then(|_| stream.flush())
        .map_err(|e| {
            if e.kind() == ErrorKind::BrokenPipe {
                DisconnectedStream(e)
            } else {
                SendBytes(e)
            }
        })
}
