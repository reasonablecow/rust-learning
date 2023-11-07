use std::{io, net::TcpStream, thread, time::Duration};

use serde::{Deserialize, Serialize};

use cli_ser::{get_host_and_port, read_msg, send_msg, Message};

#[derive(Serialize, Deserialize, Debug)]
enum Image {
    Png(Vec<u8>),
}

#[derive(Serialize, Deserialize, Debug)]
struct File {
    filename: String,
    bytes: Vec<u8>,
}

fn get_msg() -> Option<Message> {
    println!("Please type the commands:");
    let mut buffer = String::new();
    io::stdin().read_line(&mut buffer).expect("tmp");

    let string = if let Some(stripped) = buffer.strip_suffix('\n') {
        stripped.to_string()
    } else {
        buffer
    };
    Some(Message::Text(string))
}

fn main() {
    let address = get_host_and_port();
    let mut stream =
        TcpStream::connect(address).expect("Connection to the server should be possible.");

    let mut sc = stream
        .try_clone()
        .expect("The TcpStream should be cloneable.");

    let handler = thread::spawn(move || loop {
        if let Some(msg) = read_msg(&mut sc) {
            println!("{:?}", msg);
        }
        thread::sleep(Duration::from_secs(5));
    });

    while let Some(msg) = dbg!(get_msg()) {
        // let cmd = Cmd::from_stdin()?;
        // let msg = Message::from_cmd(cmd)?;
        // let msg = Message::Text(String::from("hello world"));
        send_msg(&mut stream, msg);
    }

    let res = handler.join();
    println!("{:?}", res);
}
