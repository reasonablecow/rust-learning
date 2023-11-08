use std::{
    io::{self, Read, Write},
    net::TcpStream,
    thread,
    time::Duration,
};

use regex::Regex;
use serde::{Deserialize, Serialize};

/*
#[derive(Serialize, Deserialize, Debug)]
enum Image {
    Png(Vec<u8>),
}

#[derive(Serialize, Deserialize, Debug)]
struct File {
    filename: String,
    bytes: Vec<u8>,
}
*/

/*
#[derive(Debug)]
struct Cmd {
    cmd: String,
    arg: String,
}
*/

#[derive(Serialize, Deserialize, Debug)]
pub enum Message {
    Text(String),
    //Image(Image),
    //File(String),
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
enum Command {
    Quit,
    File(String),
    Image(String),
    Text(String),
}

/// TODO
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

/// TODO
pub fn get_host_and_port() -> String {
    String::from("127.0.0.1:11111")
}

/// TODO
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
            println!("len! {:?}", len);
            let mut msg_buf = vec![0u8; len];
            stream
                .read_exact(&mut msg_buf)
                .expect("Reading the whole message should be ok.");
            let msg: Message = bincode::deserialize(&msg_buf[..])
                .expect("Deserialization of the read message should be ok.");
            println!("msg! {:?}", msg);
            Some(msg)
        }
        Err(_) => {
            // TODO
            //println!("{:?}", e),
            // Check only the errors caused by nonblocking
            // ErrorKind::UnexpectedEof => {
            None
        }
    }
}

/// Serializes Message into bytes.
///
/// !Panics if serialization fails (should never happen).
pub fn serialize_msg(msg: &Message) -> Vec<u8> {
    bincode::serialize(msg)
        .expect("Message serialization should always work - contact the implementer!")
}

pub fn send_bytes(stream: &mut TcpStream, bytes: &Vec<u8>) -> Result<(), io::Error> {
    stream.write_all(&((bytes.len() as u32).to_be_bytes()))?;
    stream.write_all(bytes)?;
    stream.flush()?;
    Ok(())
}

pub fn simulate_connections() {
    let address = get_host_and_port();

    let connection_simulator = thread::spawn(move || {
        let mut streams = Vec::new();
        for sth in ["one", "two", "three", "four", "five"] {
            let mut stream = TcpStream::connect(address.clone())
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
    fn parse_quit_ok() {
        assert_eq!(Command::Quit, parse_message(".quit"));
        assert_eq!(Command::Quit, parse_message("      .quit      "));
    }

    #[test]
    fn parse_file() {
        let f = "a.txt";
        assert_eq!(
            Command::File(String::from(f)),
            parse_message(format!(".file {}", f).as_str())
        );
    }

    #[test]
    fn parse_text() {
        for s in [". quit", ".quit      s", "a   .quit "] {
            assert_eq!(Command::Text(String::from(s)), parse_message(s));
        }
    }
}
