//! # Text Transformation Tool
//!
//! Implements requirements from
//! * <https://robot-dreams-rust.mag.wiki/7-concurrency-multithreading/index.html#homework>
//! * <https://robot-dreams-rust.mag.wiki/5-error-handling/index.html#homework>
//! * <https://robot-dreams-rust.mag.wiki/2-rust-basics/index.html#homework>
//!
//! ```sh
//! # single transformation example:
//! <example.csv cargo run csv
//! # multiple transformations example:
//! <example.txt cargo run >out.txt; echo "Errors? $?"
//! ```

use std::{env, error::Error, fs, io, sync::mpsc, thread};
use text_tool::Transformation;

/// Prints transformed standard input based on its values and arguments given.
///
/// Running the executable with an argument triggers an "one-shot" mode,
/// "interactive" mode runs otherwise.
///
/// ## One-shot Mode (single thread)
///
/// The function tries to:
/// 1. read the standard input,
/// 2. apply a transformation* to it,
/// 3. print the result to the standard output.
///
/// * It is chosen based on the argument given to the executable.
///
/// ## Interactive Mode (multi-threaded)
///
/// Parses each line of standard input into transformation and its input.
/// The rest is similar to one-shot mode.
///
/// ## Error Details
///
/// In case of an error, there is an attempt to print the error to the standard
/// error and the process returns `ExitCode::FAILURE`
/// (see <https://doc.rust-lang.org/src/std/process.rs.html#2291-2301>).
///
/// It looks like both `eprintln!` macro and `io::attempt_print_to_stderr` uses
/// `stderr().write_fmt(args)` in the end
/// (see <https://doc.rust-lang.org/src/std/io/stdio.rs.html#1039>).
fn main() -> Result<(), Box<dyn Error>> {
    if let Some(argument) = read_single_argument()? {
        // One thread, one transformation, multi-line transformation input
        let t: Transformation = argument.parse()?;
        let text = io::read_to_string(io::stdin())?;
        print!("{}", t.transform(&text)?);
        Ok(())
    } else {
        // Two threads, many transformations, single-line transformation input
        let (sender, receiver) = mpsc::channel();

        let reader = thread::spawn(move || {
            let mut buffer = String::new();
            let stdin = io::stdin();

            loop {
                buffer.clear();
                match stdin.read_line(&mut buffer) {
                    Err(msg) => break Err(msg.to_string()), // wanted to do `e @ Err(_) => return e`
                    Ok(0) => break Ok(()),                  // EOF
                    Ok(_) => {
                        if let Err(msg) = sender.send(parse_line(&buffer)) {
                            break Err(msg.to_string());
                        } else {
                            continue;
                        };
                    }
                }
            }
        });

        let editor = thread::spawn(move || {
            let mut state = Ok(());
            let general_error = Err("all errors are in the standard error");
            for content in receiver {
                match content {
                    Err(msg) => {
                        state = general_error;
                        eprintln!("{}", msg);
                    }
                    Ok((t, text)) => match t.transform(&text) {
                        Err(msg) => {
                            state = general_error;
                            eprintln!("{}", msg);
                        }
                        Ok(s) => println!("{}", s),
                    },
                }
            }
            state
        });

        match (reader.join(), editor.join()) {
            (Ok(Ok(_)), Ok(Ok(_))) => Ok(()),
            (read, edit) => Err(format!("{:?} | {:?}", read, edit).into()),
        }
    }
}

/// Returns Option of the only command line argument.
/// Giving more than one argument results in an error.
fn read_single_argument() -> Result<Option<String>, &'static str> {
    let mut args = env::args().skip(1);
    if args.len() <= 1 {
        Ok(args.next())
    } else {
        Err("More than one argument given!")
    }
}

/// Parses string into Transformation variant and an argument string*
///
/// * In Transformation::Csv case the argument is treated as a file name.
///
/// The line parsing should be equivalent to the following regex:
/// `^\s*(?<transformation>\w+) (?<argument>.*)\n?$`
/// (<https://docs.rs/regex/latest/regex/index.html#syntax>).
///
/// _In the future: Return Error object that works with Send._
fn parse_line(raw: &str) -> Result<(Transformation, String), String> {
    let without_newline = if let Some(stripped) = raw.strip_suffix('\n') {
        stripped
    } else {
        raw
    };
    if let Some((cmd, arg)) = without_newline.trim_start().split_once(' ') {
        match cmd.parse::<Transformation>() {
            Ok(tr) => match tr {
                Transformation::Csv => match fs::read_to_string(arg.trim()) {
                    Ok(csv) => Ok((tr, csv)),
                    Err(e) => Err(format!("{} | {}", e, arg)),
                },
                _ => Ok((tr, arg.to_string())),
            },
            Err(e) => Err(e.to_string()),
        }
    } else {
        Err(format!(
            "Couldn't parse into `<command> <input>`, data={:?}",
            raw
        ))
    }
}
