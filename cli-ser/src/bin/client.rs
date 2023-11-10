use std::{fs, io::Write, net::TcpStream, path::Path, sync::mpsc, thread, time::Duration};

use clap::Parser;

use cli_ser::{read_msg, send_bytes, serialize_msg, Command, Message};

/* // Dunno how to do lazy statics...
use once_cell::sync::Lazy;
static FILES_DIR: Lazy<PathBuf> = Lazy::new(|| PathBuf::from("files"));
*/

/// Client executable, interactively sends messages to the specified server.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Server host
    #[arg(long, default_value_t = String::from("127.0.0.1"))]
    host: String,

    /// Server port
    #[arg(short, long, default_value_t = 11111)]
    port: u32,

    /// Save all images as PNG.
    #[arg(short, long, default_value_t = false)]
    save_png: bool,
}

/// Client's main function.
///
/// The main thread in a loop executes commands given by the user.
/// Another thread is spawned in the background to receive messages from the server.
fn main() {
    let files_dir = Path::new("files");
    let images_dir = Path::new("images");

    let args = Args::parse();

    fs::create_dir_all(files_dir).expect("Directory for files couldn't be created.");
    fs::create_dir_all(images_dir).expect("Directory for images couldn't be created.");

    let mut stream = TcpStream::connect(format!("{}:{}", args.host, args.port))
        .expect("Connection to the server should be possible.");

    let mut stream_clone = stream
        .try_clone()
        .expect("The TcpStream should be cloneable.");

    // Channel to indicate to stop waiting for messages.
    let (end_sender, end_receiver) = mpsc::channel();

    // Reads messages from the server in the background.
    let receiver = thread::spawn(move || loop {
        if let Some(msg) = read_msg(&mut stream_clone) {
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
                    if args.save_png {
                        image.save_as_png(images_dir);
                    } else {
                        image.save(images_dir);
                    }
                    println!("Received image...");
                }
            }
        } else if end_receiver.try_recv().is_ok() {
            break;
        }
    });

    // Parses and executes commands given by the user.
    loop {
        let msg = match Command::from_stdin() {
            Command::Quit => {
                println!("Goodbye!");
                // Time for messages which were sent but were not receivable yet.
                thread::sleep(Duration::from_secs(5));
                end_sender
                    .send(true)
                    .expect("Program crashed during closing.");
                break;
            }
            cmd => Message::from_cmd(cmd).expect("User provided wrong command."),
        };
        send_bytes(&mut stream, &serialize_msg(&msg))
            .expect("Sending of you message failed, please restart and try again.");
    }

    // Wait for the receiver to finish its job.
    receiver
        .join()
        .expect("Message receiver crashed, sorry for the inconvenience.");
}
