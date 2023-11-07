use std::{
    net::{TcpListener,SocketAddr, TcpStream},
    error::Error,
    io::{Read,Write,ErrorKind, Error as IoError},
    collections::HashMap,
    thread,
};

use cli_ser::{Message, read_msg, write_msg};

// Server is reading and writing in a blocking fashion.
// let Some() = res.err().map(|x| x.kind())
fn main() -> Result<(), Box<dyn Error>> {
    println!("i am the server!");
    let (host, port) = ("127.0.0.1", "11111");

    let listener = TcpListener::bind(format!("{host}:{port}"))?;

    let mut clients: HashMap<SocketAddr, TcpStream> = HashMap::new();

    for incoming in listener.incoming() {
        let mut stream = incoming?;
        let addr = stream.peer_addr()
            .expect("Address of the TcpStream other end should be readable");
        clients.insert(addr.clone(), stream);

        let mut stream_clone = clients.get(&addr).unwrap().try_clone().unwrap();

        let handler = thread::spawn(move || {
            handle_connection(stream_clone);
        });
    }
    Ok(())
}

fn handle_connection(mut stream: TcpStream) {
    loop {
        let res = dbg!(read_msg(&mut stream));

        match res {
            Ok(msg) => {
                println!("broadcasting..."); 
                write_msg(&mut stream, msg).unwrap();
            }
            Err(err) => {
                if let Some(io_error) = err.downcast_ref::<IoError>() {
                    match io_error.kind() {
                        ErrorKind::UnexpectedEof => {
                            println!("unexpected eof {:?}", io_error);
                            break // lets go somewhere else
                        }
                        _ => {
                            println!("Other IO error: {:?}", io_error);
                        }
                    }
                } else {
                    println!("Non-IO error: {:?}", err);
                }
            }
        }
    }
}
