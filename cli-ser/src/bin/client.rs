use std::{
    error::Error,
    io::{self, ErrorKind, Write},
    net::TcpStream,
    thread,
    time::Duration,
};

use regex::Regex;
use serde::{Deserialize, Serialize};

use cli_ser::{read_msg, send_msg, Message};

#[derive(Serialize, Deserialize, Debug)]
enum Image {
    Png(Vec<u8>),
}

#[derive(Serialize, Deserialize, Debug)]
struct File {
    filename: String,
    bytes: Vec<u8>,
}

#[derive(Debug)]
struct Cmd {
    cmd: String,
    arg: String,
}

fn get_msg() -> Message {
    println!("Please type the commands:");
    let mut buffer = String::new();
    io::stdin().read_line(&mut buffer).expect("tmp");

    let string = if let Some(stripped) = buffer.strip_suffix('\n') {
        stripped.to_string()
    } else {
        buffer
    };
    Message::Text(string)
}

fn main() -> Result<(), Box<dyn Error>> {
    let (host, port) = ("127.0.0.1", "11111");
    let mut stream = TcpStream::connect(format!("{host}:{port}"))?;

    let mut sc = stream.try_clone()?;

    let handler = thread::spawn(move || loop {
        if let Some(msg) = read_msg(&mut sc) {
            println!("{:?}", msg);
        }
        thread::sleep(Duration::from_secs(5));
    });

    loop {
        // let cmd = Cmd::from_stdin()?;
        // let msg = Message::from_cmd(cmd)?;
        // let msg = Message::Text(String::from("hello world"));
        send_msg(&mut stream, dbg!(get_msg()));
    }

    let res = handler.join();
    Ok(())
}
