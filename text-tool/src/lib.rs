use core::fmt;
use regex::Regex;
use std::error::Error;
use std::str::FromStr;

pub enum Transformation {
    Lowercase,
    Uppercase,
    NoSpaces,
    Slugify,
    OneSpace,
    Csv,
}

impl Transformation {
    pub fn transform(&self, s: &str) -> Result<String, Box<dyn Error>> {
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
pub struct ParseTransformationError(String);

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
