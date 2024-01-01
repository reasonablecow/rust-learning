//! Client-Server Utilities
//!
//! TODO: buffered read and write <https://tokio.rs/tokio/tutorial/framing>
//! TODO: `struct Username(String)` with From<&str>.

use std::{
    ffi::{OsStr, OsString},
    io::{Cursor, Seek},
    path::{Path, PathBuf},
    result,
};

use async_trait::async_trait;
use chrono::{offset::Utc, SecondsFormat};
use image::ImageFormat;
use serde::{Deserialize, Serialize};
use tokio::{
    fs,
    io::{self, AsyncReadExt, AsyncWriteExt, ErrorKind},
};

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
    #[error("converting image to another type failed")]
    ConvertImg(image::error::ImageError),
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
    async fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let reader = image::io::Reader::open(path)
            .and_then(|r| r.with_guessed_format())
            .map_err(LoadImg)?;
        let format = reader.format().expect("Bug in the crate \"image \"! This should never fail when the previous step has succeeded.");

        let mut buf_reader: std::io::BufReader<std::fs::File> = reader.into_inner();
        buf_reader.seek(io::SeekFrom::Start(0)).map_err(LoadImg)?;
        let mut file: fs::File = buf_reader.into_inner().into();

        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).await.map_err(LoadImg)?;
        Ok(Image { format, bytes })
    }

    pub async fn save(&self, dir: &Path) -> Result<PathBuf> {
        let path = Self::create_path(dir, self.format);
        create_file_and_write_bytes(&path, &self.bytes)
            .await
            .map(|_| path)
            .map_err(SaveImg)
    }

    pub async fn save_as_png(self, dir: &Path) -> Result<PathBuf> {
        if self.format != ImageFormat::Png {
            let mut bytes = Vec::<u8>::new();
            image::io::Reader::with_format(Cursor::new(self.bytes), self.format)
                .decode()
                .and_then(|img| img.write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png))
                .map_err(ConvertImg)?;
            let path = Self::create_path(dir, ImageFormat::Png);
            create_file_and_write_bytes(&path, &bytes)
                .await
                .map(|_| path)
                .map_err(SaveImg)
        } else {
            self.save(dir).await
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
    async fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        let mut bytes = Vec::new();
        let mut file = fs::File::open(&path).await.map_err(LoadFile)?;
        file.read_to_end(&mut bytes).await.map_err(LoadFile)?;
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
    pub async fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        create_file_and_write_bytes(path.as_ref().join(&self.name), &self.bytes)
            .await
            .map_err(SaveFile)
    }
}

async fn create_file_and_write_bytes(path: impl AsRef<Path>, bytes: &[u8]) -> io::Result<()> {
    let mut file = fs::File::create(path).await?;
    file.write_all(bytes).await?;
    file.flush().await?;
    Ok(())
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
    pub struct User {
        pub username: String,
        pub password: String,
    }

    #[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq)]
    pub enum Auth {
        LogIn(User),
        SignUp(User),
    }

    #[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq)]
    pub enum Msg {
        Auth(Auth),
        Data(Data),
    }
    impl Msg {
        /// Loads File from a path.
        pub async fn file_from_path(path: impl AsRef<Path>) -> Result<Self> {
            File::from_path(path).await.map(Data::from).map(Self::Data)
        }

        /// Loads Image from a path.
        pub async fn img_from_path(path: impl AsRef<Path>) -> Result<Self> {
            Image::from_path(path).await.map(Data::from).map(Self::Data)
        }
    }
    impl Messageable for Msg {}
}

pub mod ser {
    use crate::*;

    #[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
    pub enum Error {
        Receiving(String),
        Sending(String),
        NotAuthenticated(cli::Msg),
        AlreadyAuthenticated(std::net::SocketAddr),
        WrongUser,
        WrongPassword,
        UsernameTaken,
    }

    #[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq)]
    pub enum Msg {
        Authenticated,
        Error(Error),
        DataFrom { data: crate::Data, from: String },
    }
    impl From<Error> for Msg {
        fn from(value: Error) -> Self {
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
