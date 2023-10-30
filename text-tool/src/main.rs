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

use core::fmt;
use regex::Regex;
use std::error::Error;
use std::str::FromStr;
use std::sync::mpsc;
use std::{env, fs, io, thread};

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

enum Transformation {
    Lowercase,
    Uppercase,
    NoSpaces,
    Slugify,
    OneSpace,
    Csv,
}

impl Transformation {
    fn transform(&self, s: &str) -> Result<String, Box<dyn Error>> {
        match self {
            Transformation::Lowercase => Ok(s.to_lowercase()),
            Transformation::Uppercase => Ok(s.to_uppercase()),
            Transformation::NoSpaces => Ok(s.replace(' ', "")),
            Transformation::Slugify => Ok(slug::slugify(s)),
            Transformation::OneSpace => {
                Ok(Regex::new(r"\s+").map(|p| p.replace_all(s, " ").to_string())?)
            }
            Transformation::Csv => Ok(Csv::from_str(s)?.to_string()),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ParseTransformationError(String);

impl fmt::Display for ParseTransformationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl Error for ParseTransformationError {}

impl FromStr for Transformation {
    type Err = ParseTransformationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().replace('-', "").as_str() {
            "lowercase" => Ok(Transformation::Lowercase),
            "uppercase" => Ok(Transformation::Uppercase),
            "nospaces" => Ok(Transformation::NoSpaces),
            "slugify" => Ok(Transformation::Slugify),
            "onespace" => Ok(Transformation::OneSpace),
            "csv" => Ok(Transformation::Csv),
            _ => Err(ParseTransformationError(format!(
                "Argument \"{}\" can not be parsed to Transformation!",
                s
            ))),
        }
    }
}

// impl From<String> for Transformation {
//     fn from(s: String) -> Self {
//         s.as_str().parse()
//     }
// }

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

/// Structure to hold CSV data.
struct Csv<'a> {
    row_length: usize,
    rows: Vec<Vec<&'a str>>,
}

impl Csv<'_> {
    /// Parses well-formed CSV from borrowed str.
    fn from_str(s: &str) -> Result<Csv, Box<dyn Error>> {
        // Split string into rows and rows into fields.
        let csv = s
            .lines()
            .map(|line| line.split(',').map(|s| s.trim()).collect::<Vec<_>>())
            .collect::<Vec<_>>();

        // Check all rows have the same number of fields.
        let mut row_lengths = csv.iter().map(|h| h.len());
        let Some(hdr_len) = row_lengths.next() else {
            return Err("CSV must have an header.".into());
        };
        let true = row_lengths.all(|l| l == hdr_len) else {
            return Err("Every CSV row must have the same number of fields as the header.".into());
        };

        Ok(Csv {
            row_length: hdr_len,
            rows: csv,
        })
    }
}

impl fmt::Display for Csv<'_> {
    /// Formats Csv as text table.
    /// **Warning!**: Line endings are always LF byte.
    /// If CRLF is desired use `s.replace("\n", "\r\n")`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Calculate width for each field.
        let Some(widths) = (0..self.row_length)
            .map(|field_idx| self.rows.iter().map(|row| row[field_idx].len()).max())
            .collect::<Option<Vec<_>>>()
        else {
            // CSV calculation of column widths failed, contact the implementer.
            return Err(fmt::Error);
        };
        // Pad fields with spaces, join fields with "|", join lines with "\n".
        write!(
            f,
            "{}",
            self.rows
                .iter()
                .map(|row| {
                    format!(
                        "|{}|\n",
                        row.iter()
                            .zip(widths.iter())
                            .map(|(field, width)| format!(" {:width$} ", field, width = width))
                            .collect::<Vec<_>>()
                            .join("|")
                    )
                })
                .collect::<Vec<_>>()
                .concat()
        )
    }
}
