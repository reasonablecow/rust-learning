//! [Text Transformation](https://robot-dreams-rust.mag.wiki/2-rust-basics/index.html#homework)

use std::env;
use std::io;
use std::io::Read; // Needed for Stdin.read_to_string
use std::process;

fn main() {
    let transform = choose_transformation();
    let text = read_input();
    print!("{}", transform(text.as_str()));
}

/// Choose transformation function based on command argument given.
fn choose_transformation() -> fn(&str) -> String {
    match read_argument().as_str() {
        "lowercase" => str::to_lowercase,
        "uppercase" => str::to_uppercase,
        "no-spaces" => |s: &str| -> String { s.replace(' ', "") },
        // "slugify" => slug::slugify, // Does not work :/
        "slugify" => |s: &str| -> String { slug::slugify(s) },
        _ => panic!("Error: unsupported argument given!"),
    }
}

fn read_argument() -> String {
    let mut args = env::args().skip(1);
    let Some(arg) = args.next() else {
        error("Error: missing required argument!");
    };
    if args.len() > 0 {
        error("Error: more than one argument given!");
    }
    arg
}

fn error(msg: &str) -> ! {
    eprintln!("{}", msg);
    process::exit(1);
}

/// Read a string from standard input.
fn read_input() -> String {
    let mut s = String::new();
    io::stdin()
        .read_to_string(&mut s)
        .expect("standard input should be readable");
    s
}
