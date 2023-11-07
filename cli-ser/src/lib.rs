use std::{
    error::Error,
    net::TcpStream,
    io::{Write, Read},
};

use regex::Regex;
use serde::{Serialize, Deserialize};

// #[derive(Serialize, Deserialize, Debug)]
// enum Image {
//     Png(Vec<u8>),
// }
// 
// #[derive(Serialize, Deserialize, Debug)]
// struct File {
//     filename: String,
//     bytes: Vec<u8>,
// }

#[derive(Serialize, Deserialize, Debug)]
pub enum Message {
    Text(String),
    //Image(Image),
    File(String),
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
enum Command {
    Quit,
    File(String),
    Image(String),
    Text(String),
}

fn parse_message(s: &str) -> Command {
    let err_msg = "unexpected regex error, contact the crate implementer";
    //let re = Regex::new(r"(?m)^([^:]+):([0-9]+):(.+)$").unwrap();
    //let r = Regex::new(r"\.file\s*(?<path>\w+)").expect();
    let reg_quit = Regex::new(r"^\s*\.quit\s*$").expect(err_msg);
    let reg_file = Regex::new(r"^\s*\.file\s+(?<file>\S+.*)\s*$").expect(err_msg);

    if reg_quit.is_match(s) {
        Command::Quit
    } else if let Some((_, [file])) = reg_file.captures(s).map(|caps| caps.extract()) {
        Command::File(file.to_string())
    } else {
        Command::Text(s.to_string())
    }
    // let Some(caps) = re.captures(s) else {
    //     println!("no match!");
    //     return;
    // };
}

pub fn read_msg(stream: &mut TcpStream) -> Result<Message, Box<dyn Error>> {
    let addr = stream.peer_addr().unwrap();

    let mut len_bytes = [0u8; 4];
    stream.read_exact(&mut len_bytes)?;
    let len = u32::from_be_bytes(len_bytes) as usize;
    println!("len! {:?}", len);

    let mut bytes = vec![0u8; len];
    stream.read_exact(&mut bytes)?;
    println!("buf! {:?}", bytes);

    let decoded: Message = bincode::deserialize(&bytes[..])?;
    println!("dec! {:?}", decoded);
    Ok(decoded)
}

pub fn write_msg(stream: &mut TcpStream, msg: Message) -> Result<(), Box<dyn Error>> {
    let bytes = bincode::serialize(&msg)?;
    let len = bytes.len() as u32;
    stream.write(&len.to_be_bytes())?;

    stream.write_all(&bytes)?;
    stream.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_quit_ok() {
        assert_eq!(Command::Quit, parse_message(".quit"));
        assert_eq!(Command::Quit, parse_message("      .quit      "));
    }

    #[test]
    fn parse_file() {
        let f = "a.txt";
        assert_eq!(Command::File(String::from(f)), parse_message(format!(".file {}", f).as_str()));
    }

    #[test]
    fn parse_text() {
        for s in [". quit", ".quit      s", "a   .quit "] {
            assert_eq!(Command::Text(String::from(s)), parse_message(s));
        }
    }
}
