use std::{fs, io::Write, net::TcpStream, thread, time::Duration};

use cli_ser::{get_host_and_port, read_msg, send_bytes, serialize_msg, Command, Message};

const FILES_DIR: &str = "files";

fn main() {
    let address = get_host_and_port();
    let mut stream =
        TcpStream::connect(address).expect("Connection to the server should be possible.");

    let mut sc = stream
        .try_clone()
        .expect("The TcpStream should be cloneable.");

    let _receiver = thread::spawn(move || loop {
        if let Some(msg) = read_msg(&mut sc) {
            match msg {
                Message::File(f) => {
                    println!("Received {}", f.name);
                    fs::create_dir_all(FILES_DIR)
                        .expect("Directory for files couldn't be created.");
                    let mut file = fs::File::create(format!("{}/{}", FILES_DIR, f.name)) // TODO use path utilities instead of unix conventions.
                        .expect("File creation failed.");
                    file.write_all(&f.bytes).expect("Writing the file failed.");
                }
                Message::Text(text) => println!("{}", text),
            }
        }
        thread::sleep(Duration::from_millis(100));
    });

    loop {
        let msg = match Command::from_stdin() {
            Command::Quit => {
                println!("Goodbye!");
                break;
            }
            cmd => Message::from_cmd(cmd).expect("User provided wrong command."),
        };
        println!("sending {:?}", msg);
        send_bytes(&mut stream, &serialize_msg(&msg))
            .expect("Sending of you message failed, please restart and try again.");
    }
}
