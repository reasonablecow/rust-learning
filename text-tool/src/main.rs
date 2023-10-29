//! # Text Transformation Tool
//!
//! Implements requirements from
//! * <https://robot-dreams-rust.mag.wiki/7-concurrency-multithreading/index.html#homework>
//! * <https://robot-dreams-rust.mag.wiki/5-error-handling/index.html#homework>
//! * <https://robot-dreams-rust.mag.wiki/2-rust-basics/index.html#homework>
//!
//! Try this out `<example.csv cargo run csv`.

use core::fmt;
use regex::Regex;
use std::error::Error;
use std::{env, io};

/// When there are no errors it reads the standard input, transforms it
/// and prints the result to the standard output.
///
/// The transformation is chosen based on command argument.
///
/// In case of an error, there is an attempt to print the error to the standard
/// error and the process returns `ExitCode::FAILURE`
/// (see <https://doc.rust-lang.org/src/std/process.rs.html#2291-2301>).
///
/// It looks like both `eprintln!` macro and `io::attempt_print_to_stderr` uses
/// `stderr().write_fmt(args)` in the end
/// (see <https://doc.rust-lang.org/src/std/io/stdio.rs.html#1039>).
fn main() -> Result<(), Box<dyn Error>> {
    // TODO: transformation variants change to enum, implement FromStr trait
    // TODO: interactive: spawn threads input-parser and string-transformer
    // TODO: interactive: CSV function change read string from path
    if let Some(argument) = read_single_argument()? {
        let transform = parse_transformation(&argument)?;
        let text = io::read_to_string(io::stdin())?;
        print!("{}", transform(&text)?);
        Ok(())
    } else { // TODO:
        //spawn_reader
        //    send
        //        parse_input
        //            read_stdin
        //spawn_writer
        //    for transform, text in recv
        //        print
        //            transform
        //                text
        todo!()
    }
}

type FnStrToResult = fn(&str) -> Result<String, Box<dyn Error>>;

/// Returns transformation function based on given argument.
fn parse_transformation(argument: &str) -> Result<FnStrToResult, String> {
    Ok(match argument {
        "lowercase" => |s| Ok(s.to_lowercase()),
        "uppercase" => |s| Ok(s.to_uppercase()),
        "no-spaces" => |s| Ok(s.replace(' ', "")),
        "slugify" => |s| Ok(slug::slugify(s)),
        "one-space" => |s| Ok(Regex::new(r"\s+").map(|p| p.replace_all(s, " ").to_string())?),
        "csv" => |s| Ok(Csv::from_str(s)?.to_string()),
        other => return Err(format!("Argument \"{}\" is not supported!", other)),
    })
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
