//! [Text Transformation](https://robot-dreams-rust.mag.wiki/2-rust-basics/index.html#homework)

use regex::Regex;
use std::env;
use std::error::Error;
use std::io;
use std::io::Read; // Needed for Stdin.read_to_string

fn main() -> Result<(), Box<dyn Error>> {
    let transform = choose_transformation()?;
    let text = read_input()?;
    Ok(print!("{}", transform(text.as_str())))
}

/// Choose transformation function based on command argument given.
fn choose_transformation() -> Result<fn(&str) -> String, String> {
    Ok(match read_single_argument()?.as_str() {
        "lowercase" => str::to_lowercase,
        "uppercase" => str::to_uppercase,
        "no-spaces" => |s: &str| -> String { s.replace(' ', "") },
        // "slugify" => slug::slugify, // Does not work :/
        "slugify" => |s: &str| -> String { slug::slugify(s) },
        "one-space" => {
            |s: &str| -> String { Regex::new(r"\s+").unwrap().replace_all(s, " ").to_string() }
        }
        other => return Err(format!("Argument \"{}\" is not supported!", other)),
    })
}

/// Returns the command line argument if only one given,
/// otherwise returns an error.
fn read_single_argument() -> Result<String, String> {
    let mut args = env::args().skip(1);
    if let Some(arg) = args.next() {
        if args.len() == 0 {
            Ok(arg)
        } else {
            Err(String::from("More than one argument given!"))
        }
    } else {
        Err(String::from("Missing required argument!"))
    }
}

/// Read a string from standard input.
fn read_input() -> io::Result<String> {
    let mut text = String::new();
    io::stdin().read_to_string(&mut text)?;
    Ok(text)
}
