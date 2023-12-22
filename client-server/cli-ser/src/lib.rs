//! Client-Server Utilities

use std::{
    ffi::{OsStr, OsString},
    fs,
    io::{self, Cursor, ErrorKind, Read, Write},
    path::{Path, PathBuf},
    result,
};

use async_trait::async_trait;
use chrono::{offset::Utc, SecondsFormat};
use image::ImageFormat;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

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

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
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

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
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

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum ServerErr {
    Receiving(String),
    Sending(String),
}

/// Data to be sent over the network.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum Data {
    Text(String),
    File(File),
    Image(Image),
}
impl From<File> for Data {
    fn from(value: File) -> Data {
        Data::File(value)
    }
}
impl From<Image> for Data {
    fn from(value: Image) -> Data {
        Data::Image(value)
    }
}

pub mod cli {
    use crate::*;
    use std::path::Path;

    #[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq)]
    pub enum Msg {
        Auth { username: String, password: String },
        Data(Data),
    }
    impl Msg {
        /// Loads File from a path.
        pub fn file_from_path(path: impl AsRef<Path>) -> Result<Self> {
            File::from_path(path).map(Data::from).map(Self::Data)
        }

        /// Loads Image from a path.
        pub fn img_from_path(path: impl AsRef<Path>) -> Result<Self> {
            Image::from_path(path).map(Data::from).map(Self::Data)
        }
    }
    impl Messageable for Msg {}
}

pub mod ser {
    use crate::*;

    #[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq)]
    pub enum Msg {
        Info(String),
        Error(ServerErr),
        DataFrom { data: crate::Data, from: String },
    }
    impl From<ServerErr> for Msg {
        fn from(value: ServerErr) -> Self {
            Msg::Error(value)
        }
    }
    impl Messageable for Msg {}
}

#[async_trait]
pub trait Messageable
where
    Self: serde::ser::Serialize,
    for<'de> Self: serde::de::Deserialize<'de>,
{
    /// Serializes Message into bytes.
    fn to_bytes(&self) -> Result<Vec<u8>> {
        bincode::serialize(self).map_err(SerializeMsg)
    }

    /// Deserialize Message from bytes.
    fn from_bytes(bytes: &[u8]) -> Result<Self> {
        bincode::deserialize(bytes).map_err(DeserializeMsg)
    }

    /// Tries to receive a message from the given stream.
    async fn receive<S>(stream: &mut S) -> Result<Self>
    where
        S: AsyncReadExt + std::marker::Unpin + std::marker::Send,
    {
        Self::from_bytes(&read_bytes(stream).await?)
    }

    /// Sends a message over the given stream.
    async fn send<S>(&self, socket: &mut S) -> Result<()>
    where
        S: AsyncWriteExt + std::marker::Unpin + std::marker::Send,
    {
        write_bytes(socket, &self.to_bytes()?).await
    }
}

pub async fn read_bytes(stream: &mut (impl AsyncReadExt + std::marker::Unpin)) -> Result<Vec<u8>> {
    fn map_err(e: io::Error) -> Error {
        if e.kind() == ErrorKind::UnexpectedEof {
            DisconnectedStream(e)
        } else {
            ReceiveBytes(e)
        }
    }
    let len = stream.read_u32().await.map_err(map_err)?;
    let mut bytes = vec![0u8; len as usize];
    stream.read_exact(&mut bytes).await.map_err(map_err)?;
    Ok(bytes)
}

/// todo: tried to use future.and_then, but the writer was borrowed multiple times...
pub async fn write_bytes(
    writer: &mut (impl AsyncWriteExt + std::marker::Unpin),
    bytes: &[u8],
) -> Result<()> {
    fn map_err(e: io::Error) -> Error {
        if e.kind() == ErrorKind::BrokenPipe {
            DisconnectedStream(e)
        } else {
            SendBytes(e)
        }
    }

    writer
        .write_u32(bytes.len() as u32)
        .await
        .map_err(map_err)?;
    writer.write_all(bytes).await.map_err(map_err)?;
    writer.flush().await.map_err(map_err)?;
    Ok(())
}
