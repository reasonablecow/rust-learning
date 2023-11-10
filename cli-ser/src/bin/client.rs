use std::{fs, io::Write, net::TcpStream, path::Path, thread, time::Duration};

use cli_ser::{get_host_and_port, read_msg, send_bytes, serialize_msg, Command, Message};

/* // Dunno how to do lazy statics...
use once_cell::sync::Lazy;
static FILES_DIR: Lazy<PathBuf> = Lazy::new(|| PathBuf::from("files"));
*/

fn main() {
    let save_as_png = true; // TODO
    let files_dir = Path::new("files");
    let images_dir = Path::new("images");
    fs::create_dir_all(files_dir).expect("Directory for files couldn't be created.");
    fs::create_dir_all(images_dir).expect("Directory for images couldn't be created.");

    let address = get_host_and_port();
    let mut stream =
        TcpStream::connect(address).expect("Connection to the server should be possible.");

    let mut sc = stream
        .try_clone()
        .expect("The TcpStream should be cloneable.");

    let _receiver = thread::spawn(move || loop {
        if let Some(msg) = read_msg(&mut sc) {
            match msg {
                Message::Text(text) => println!("{}", text),
                Message::File(f) => {
                    println!("Received {:?}", f.name);
                    let path = files_dir.join(f.name);
                    fs::File::create(path)
                        .expect("File creation failed.")
                        .write_all(&f.bytes)
                        .expect("Writing the file failed.");
                }
                Message::Image(image) => {
                    if save_as_png {
                        image.save_as_png(images_dir);
                    } else {
                        image.save(images_dir);
                    }
                    println!("Received image...");
                }
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
        send_bytes(&mut stream, &serialize_msg(&msg))
            .expect("Sending of you message failed, please restart and try again.");
    }
}
